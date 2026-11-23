//! `AppendLog`: the writer of an append-only JSONL file, the book of record
//! first among them (SDD-RUST §4.0 rule 5, §4.5; invariant 5).
//!
//! Contract:
//! - **Append only.** Every write is one line, opened for append, never a
//!   read-modify-write. Each append is synced to disk before it returns, so a
//!   line the caller was told about survives a power cut: measured at 1.02 ms
//!   per line against 0.22 ms unsynced (win32, 1000 appends, 2026-10-20).
//! - **A torn last line is left alone.** A process killed mid-write leaves a
//!   line with no newline. It stays on disk exactly as it is, the reader skips
//!   it, and the next append starts on a fresh line.
//! - **`seq` is recovered at open**: the highest `seq` readable in the live
//!   file, or, when the live file has none (a restart just after a rotation),
//!   in the newest sealed segment. The writer stamps it, so a caller cannot
//!   collide with or reorder history.
//! - **Rotation is a rename and nothing else.** Before an append, a live file
//!   past its threshold is renamed into the archive as `log-<first seq>.jsonl`,
//!   so the segments and the live file concatenate to the same bytes in the same
//!   order. A rotation that cannot happen is skipped, and the append still
//!   happens: a large file is a performance problem, a lost line a correctness
//!   one. A skipped rotation is returned to the caller, never swallowed
//!   (invariant 7).
//!
//! The line's format is not this crate's: a `seq_of` function reads the `seq`
//! from a line, and the caller renders each line given its `seq`. One process
//! writes a log (ADR-0004); `&mut self` keeps one writer per handle.
//!
//! [`append_line`] is the same append for a file whose lines carry no `seq` and
//! which never rotates, such as the task archive (SDD-RUST §4.5). It keeps the
//! first two rules and reads only the file's last byte, so its cost does not
//! grow with the file (Architect, 2026-10-25).

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The live file's size at which the next append seals it (today's figure:
/// about 45 ms to parse on the measured machine, SDD-RUST §4.5).
pub const ROTATE_AT_BYTES: u64 = 4 * 1024 * 1024;

/// Reads the `seq` from one line, or `None` for a line it cannot read.
pub type SeqOf = fn(&str) -> Option<u64>;

/// The segment file a sealed live file is renamed to: the first `seq` it holds,
/// zero-padded to twelve digits so a listing sorts.
pub fn segment_name(first_seq: u64) -> String {
    format!("log-{first_seq:012}.jsonl")
}

/// The first `seq` a segment's file name encodes, or `None` if it is not one.
pub fn segment_first_seq(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("log-")?.strip_suffix(".jsonl")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|seq| *seq > 0)
}

/// What became of the rotation an append checked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rotation {
    /// The live file was under its threshold.
    NotDue,
    /// The live file was sealed as this segment.
    Sealed(PathBuf),
    /// It was due and could not happen; the append went to the live file.
    Skipped(String),
}

/// One append's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appended {
    /// The `seq` the line carries.
    pub seq: u64,
    /// The rotation checked before it.
    pub rotation: Rotation,
}

/// Why an append wrote nothing.
#[derive(Debug)]
pub enum AppendError<E> {
    /// The caller's renderer refused.
    Render(E),
    /// The rendered line holds a newline, which would make it two lines.
    Multiline,
    /// The file could not be written.
    Io(io::Error),
}

/// The writer of one append-only file and its archive.
#[derive(Debug)]
pub struct AppendLog {
    live: PathBuf,
    archive: PathBuf,
    rotate_at: u64,
    seq_of: SeqOf,
    last_seq: u64,
}

impl AppendLog {
    /// Opens the log at `live`, sealing into `archive` past `rotate_at` bytes,
    /// and recovers its `seq` (see the module contract). Creates `live`'s
    /// directory; does not create the file until the first append.
    pub fn open(
        live: &Path,
        archive: &Path,
        rotate_at: u64,
        seq_of: SeqOf,
    ) -> io::Result<AppendLog> {
        if let Some(dir) = live.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut last_seq = highest_seq(live, seq_of)?;
        if last_seq.is_none() {
            last_seq = match newest_segment(archive)? {
                Some(segment) => highest_seq(&segment, seq_of)?,
                None => None,
            };
        }
        Ok(AppendLog {
            live: live.to_path_buf(),
            archive: archive.to_path_buf(),
            rotate_at,
            seq_of,
            last_seq: last_seq.unwrap_or(0),
        })
    }

    /// The `seq` the next append will carry.
    pub fn next_seq(&self) -> u64 {
        self.last_seq + 1
    }

    /// Appends the line `render` makes for the next `seq`. See the module
    /// contract for rotation and the torn tail.
    pub fn append<E>(
        &mut self,
        render: impl FnOnce(u64) -> Result<String, E>,
    ) -> Result<Appended, AppendError<E>> {
        let seq = self.next_seq();
        let line = render(seq).map_err(AppendError::Render)?;
        if line.contains('\n') || line.contains('\r') {
            return Err(AppendError::Multiline);
        }
        let rotation = self.rotate_if_due();
        write_line(&self.live, &line).map_err(AppendError::Io)?;
        self.last_seq = seq;
        Ok(Appended { seq, rotation })
    }

    fn rotate_if_due(&self) -> Rotation {
        let size = match fs::metadata(&self.live) {
            Ok(meta) => meta.len(),
            Err(_) => return Rotation::NotDue,
        };
        if size < self.rotate_at {
            return Rotation::NotDue;
        }
        let first = match first_seq(&self.live, self.seq_of) {
            Ok(Some(first)) => first,
            Ok(None) => {
                return Rotation::Skipped(
                    "the live file holds no readable line to name a segment by".to_string(),
                );
            }
            Err(error) => return Rotation::Skipped(format!("cannot read the live file: {error}")),
        };
        let target = self.archive.join(segment_name(first));
        if target.exists() {
            return Rotation::Skipped(format!(
                "{} already exists, and sealing onto it would rewrite history",
                target.display()
            ));
        }
        match fs::create_dir_all(&self.archive).and_then(|()| fs::rename(&self.live, &target)) {
            Ok(()) => Rotation::Sealed(target),
            Err(error) => Rotation::Skipped(format!("cannot seal the live file: {error}")),
        }
    }
}

/// Appends `line` to the append-only file at `path` as one line, synced before
/// it returns, for a file whose lines carry no `seq` and which never rotates.
/// A torn last line is left as it is and `line` starts on a fresh one. Creates
/// the file, not its directory. A `line` holding a line break is refused with
/// [`io::ErrorKind::InvalidInput`], and nothing is written.
pub fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if line.contains('\n') || line.contains('\r') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the line holds a line break, which would make it two lines",
        ));
    }
    write_line(path, line)
}

/// Appends `line` and a newline, after a newline of its own when the file ends
/// mid-line.
fn write_line(path: &Path, line: &str) -> io::Result<()> {
    let torn = ends_mid_line(path)?;
    let mut text = String::with_capacity(line.len() + 2);
    if torn {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    append_bytes(path, text.as_bytes())
}

#[expect(
    clippy::disallowed_methods,
    reason = "fs-write: the append-only writer, opened for append and never truncated (invariant 5)"
)]
fn append_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_data()
}

/// Whether the file's last byte is not a newline: a torn line from a killed
/// writer. An absent or empty file is not torn.
fn ends_mid_line(path: &Path) -> io::Result<bool> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

/// Every line of `path` as text, lossily: a line torn mid-character still
/// reads as a line the `seq_of` function refuses. An absent file has none.
fn lines_of(path: &Path) -> io::Result<Vec<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_string)
            .collect()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn highest_seq(path: &Path, seq_of: SeqOf) -> io::Result<Option<u64>> {
    Ok(lines_of(path)?.iter().filter_map(|line| seq_of(line)).max())
}

fn first_seq(path: &Path, seq_of: SeqOf) -> io::Result<Option<u64>> {
    Ok(lines_of(path)?.iter().find_map(|line| seq_of(line)))
}

/// The sealed segment with the highest first `seq`, if the archive has one.
fn newest_segment(archive: &Path) -> io::Result<Option<PathBuf>> {
    let entries = match fs::read_dir(archive) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut segments = Vec::new();
    for entry in entries {
        let entry = entry?;
        if let Some(first) = segment_first_seq(&entry.file_name().to_string_lossy()) {
            segments.push((first, entry.path()));
        }
    }
    Ok(segments
        .into_iter()
        .max_by_key(|(first, _)| *first)
        .map(|(_, path)| path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic::write_atomic;

    /// A line of the form `{"seq":N}`; anything else is unreadable.
    fn seq_of(line: &str) -> Option<u64> {
        line.strip_prefix("{\"seq\":")?
            .strip_suffix('}')?
            .parse()
            .ok()
    }

    fn line(seq: u64) -> Result<String, String> {
        Ok(format!("{{\"seq\":{seq}}}"))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-fs-append-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(dir: &Path, rotate_at: u64) -> AppendLog {
        AppendLog::open(
            &dir.join("log.jsonl"),
            &dir.join("log-archive"),
            rotate_at,
            seq_of,
        )
        .unwrap()
    }

    #[test]
    fn appends_number_from_one_and_survive_a_reopen() {
        let dir = scratch("number");
        let mut log = open(&dir, ROTATE_AT_BYTES);
        assert_eq!(log.next_seq(), 1);
        for expected in 1..=3 {
            let appended = log.append(line).unwrap();
            assert_eq!(
                appended,
                Appended {
                    seq: expected,
                    rotation: Rotation::NotDue
                }
            );
        }
        let log = open(&dir, ROTATE_AT_BYTES);
        assert_eq!(log.next_seq(), 4, "seq is recovered from the file");
        assert_eq!(
            fs::read_to_string(dir.join("log.jsonl")).unwrap(),
            "{\"seq\":1}\n{\"seq\":2}\n{\"seq\":3}\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_live_file_is_not_torn() {
        let dir = scratch("empty");
        let live = dir.join("log.jsonl");
        write_atomic(&live, b"").unwrap();
        let mut log = open(&dir, ROTATE_AT_BYTES);
        log.append(line).unwrap();
        assert_eq!(
            fs::read_to_string(&live).unwrap(),
            "{\"seq\":1}\n",
            "no blank line is written before the first"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_last_line_is_left_alone_and_the_next_append_starts_fresh() {
        let dir = scratch("torn");
        let live = dir.join("log.jsonl");
        write_atomic(&live, b"{\"seq\":1}\n{\"seq\":2}\n{\"se").unwrap();
        let mut log = open(&dir, ROTATE_AT_BYTES);
        assert_eq!(log.next_seq(), 3, "the torn line is not counted");
        log.append(line).unwrap();
        assert_eq!(
            fs::read_to_string(&live).unwrap(),
            "{\"seq\":1}\n{\"seq\":2}\n{\"se\n{\"seq\":3}\n",
            "the torn bytes stay exactly as they were"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_line_torn_mid_character_reads_as_unreadable_not_as_an_error() {
        let dir = scratch("utf8");
        let live = dir.join("log.jsonl");
        let mut bytes = b"{\"seq\":4}\n".to_vec();
        bytes.extend_from_slice(&"{\"seq\":5,\"detail\":\"é".as_bytes()[..20]);
        write_atomic(&live, &bytes).unwrap();
        assert_eq!(open(&dir, ROTATE_AT_BYTES).next_seq(), 5);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_full_live_file_is_sealed_by_its_first_seq_before_the_next_append() {
        let dir = scratch("rotate");
        let mut log = open(&dir, 20);
        log.append(line).unwrap();
        log.append(line).unwrap();
        let third = log.append(line).unwrap();
        let segment = dir.join("log-archive").join("log-000000000001.jsonl");
        assert_eq!(third.rotation, Rotation::Sealed(segment.clone()));
        assert_eq!(
            fs::read_to_string(&segment).unwrap(),
            "{\"seq\":1}\n{\"seq\":2}\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("log.jsonl")).unwrap(),
            "{\"seq\":3}\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_restart_just_after_a_rotation_recovers_seq_from_the_newest_segment() {
        let dir = scratch("recover");
        let mut log = open(&dir, 20);
        for _ in 0..5 {
            log.append(line).unwrap();
        }
        // Seal the live file by hand so the live file is gone, as after a
        // rotation followed by a crash before the next append.
        let archive = dir.join("log-archive");
        fs::rename(dir.join("log.jsonl"), archive.join(segment_name(5))).unwrap();
        let log = open(&dir, 20);
        assert_eq!(
            log.next_seq(),
            6,
            "numbering continues instead of restarting at 1"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rotation_that_cannot_happen_is_reported_and_the_line_still_lands() {
        let dir = scratch("skip");
        let live = dir.join("log.jsonl");
        // No readable line: nothing to name a segment by.
        write_atomic(&live, b"not a line we can read\n").unwrap();
        let mut log = open(&dir, 1);
        let appended = log.append(line).unwrap();
        assert!(
            matches!(&appended.rotation, Rotation::Skipped(why) if why.contains("no readable line")),
            "{appended:?}"
        );
        // A segment already holding that name is never written over.
        let archive = dir.join("log-archive");
        fs::create_dir_all(&archive).unwrap();
        write_atomic(&archive.join(segment_name(1)), b"history\n").unwrap();
        let appended = log.append(line).unwrap();
        assert!(
            matches!(&appended.rotation, Rotation::Skipped(why) if why.contains("would rewrite history")),
            "{appended:?}"
        );
        assert_eq!(
            fs::read_to_string(archive.join(segment_name(1))).unwrap(),
            "history\n"
        );
        assert_eq!(
            fs::read_to_string(&live).unwrap(),
            "not a line we can read\n{\"seq\":1}\n{\"seq\":2}\n",
            "every line landed in the live file"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refused_or_multiline_render_writes_nothing() {
        let dir = scratch("refuse");
        let mut log = open(&dir, ROTATE_AT_BYTES);
        assert!(matches!(
            log.append(|_| Err::<String, _>("no")),
            Err(AppendError::Render("no"))
        ));
        assert!(matches!(
            log.append(|_| Ok::<_, ()>("two\nlines".to_string())),
            Err(AppendError::Multiline)
        ));
        assert!(matches!(
            log.append(|_| Ok::<_, ()>("carriage\rreturn".to_string())),
            Err(AppendError::Multiline)
        ));
        assert!(!dir.join("log.jsonl").exists(), "nothing was written");
        assert_eq!(log.next_seq(), 1, "no seq was spent");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_line_with_no_seq_is_appended_whole_after_a_torn_tail() {
        let dir = scratch("line");
        let file = dir.join("archive.jsonl");
        append_line(&file, "{\"a\":1}").unwrap();
        append_line(&file, "{\"b\":2}").unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\"a\":1}\n{\"b\":2}\n",
            "an absent file is created, and each line ends with a newline"
        );
        write_atomic(&file, b"{\"a\":1}\n{\"b\"").unwrap();
        append_line(&file, "{\"c\":3}").unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\"a\":1}\n{\"b\"\n{\"c\":3}\n",
            "the torn bytes stay exactly as they were"
        );
        write_atomic(&file, b"").unwrap();
        append_line(&file, "{\"d\":4}").unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\"d\":4}\n",
            "an empty file is not torn"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_line_with_a_line_break_or_no_directory_writes_nothing() {
        let dir = scratch("line-refused");
        let file = dir.join("archive.jsonl");
        for line in ["two\nlines", "carriage\rreturn"] {
            let error = append_line(&file, line).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{line:?}");
        }
        assert!(!file.exists(), "nothing was written");
        assert!(
            append_line(&dir.join("absent").join("archive.jsonl"), "{}").is_err(),
            "the directory is not created"
        );
        assert!(!dir.join("absent").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_rotation_threshold_is_today_s_four_mebibytes() {
        // Pinned, as the TypeScript build pinned it, so a test's lower value
        // cannot quietly become the shipped one (mutation round 1, 2026-10-20).
        assert_eq!(ROTATE_AT_BYTES, 4_194_304);
    }

    #[test]
    fn a_read_error_other_than_absence_is_an_error_not_an_empty_file() {
        // Mutation round 1 (2026-10-20): each not-found guard read every
        // other error as "absent" without a test noticing.
        let dir = scratch("errors");
        let not_a_file = dir.join("a-directory");
        fs::create_dir_all(&not_a_file).unwrap();
        assert!(
            lines_of(&not_a_file).is_err(),
            "a directory is not an empty log"
        );
        assert!(
            ends_mid_line(&not_a_file).is_err(),
            "nor a log that is not torn"
        );
        let not_a_dir = dir.join("a-file");
        write_atomic(&not_a_dir, b"x").unwrap();
        assert!(
            newest_segment(&not_a_dir).is_err(),
            "a file is not an empty archive"
        );
        assert!(
            AppendLog::open(&not_a_file, &dir.join("archive"), ROTATE_AT_BYTES, seq_of).is_err(),
            "a log that cannot be read is not opened as empty"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn segment_names_sort_and_read_back() {
        assert_eq!(segment_name(1), "log-000000000001.jsonl");
        assert_eq!(segment_first_seq("log-000000000001.jsonl"), Some(1));
        assert_eq!(
            segment_first_seq(&segment_name(987_654_321_012)),
            Some(987_654_321_012)
        );
        for not in [
            "log-0.jsonl",
            "log-.jsonl",
            "log-12a.jsonl",
            "log-1.json",
            "x-1.jsonl",
            "log-+1.jsonl",
        ] {
            assert_eq!(segment_first_seq(not), None, "{not}");
        }
    }

    #[test]
    fn the_newest_segment_is_the_one_with_the_highest_first_seq() {
        let dir = scratch("newest");
        let archive = dir.join("log-archive");
        fs::create_dir_all(&archive).unwrap();
        for first in [1u64, 30, 7] {
            write_atomic(
                &archive.join(segment_name(first)),
                format!("{{\"seq\":{first}}}\n").as_bytes(),
            )
            .unwrap();
        }
        write_atomic(&archive.join("notes.txt"), b"x").unwrap();
        assert_eq!(
            newest_segment(&archive).unwrap(),
            Some(archive.join(segment_name(30)))
        );
        assert_eq!(newest_segment(&dir.join("absent")).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}

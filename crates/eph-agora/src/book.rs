//! `Book`: the book of record, `log.jsonl` and its sealed segments in
//! `log-archive/` (SDD-RUST §4.5; FR-4.4, NFR-7, NFR-13; invariant 5).
//!
//! Contract:
//! - **One writer, one lock** (DECISIONS-LOG 2026-10-25, *R1.4b — Architect
//!   decisions*, item 3). [`Book::append`] takes the lock, stamps the next
//!   `seq` and the time, appends the line through `eph-fs`'s [`AppendLog`],
//!   which syncs it, then publishes it, then returns. A line the caller was
//!   told about is on disk, and appenders queue behind each other.
//! - **Publish after the line is on disk, in file order** (item 1). Each
//!   subscriber has its own channel ([`Book::subscribe`]). A line is sent to
//!   every receiver while the lock is held, so receivers see the file's order.
//!   A dropped receiver unsubscribes itself, and no subscriber can block or fail
//!   an append. A receiver nobody reads grows its queue, a recorded residual
//!   until R4 gives that case a consumer.
//! - **A reader's cursor is one byte offset into the book read as one stream**
//!   (item 2): the sealed segments, oldest first, then the live file. Rotation
//!   is a rename, so a sealed segment holds exactly the bytes the live file did,
//!   and a cursor taken before a rotation still points at the same line after
//!   it. `seq` is not the cursor: a second harness on one home repeats it.
//! - **A torn last line is skipped, and damage is counted** (DECISIONS-LOG
//!   2026-10-25, *R1.4b — Architect decisions, second round*, item 5). The live
//!   file's last bytes, when they end mid-line, are a write in progress or a
//!   kill's leftover: a read skips them, says so ([`Read::torn_tail`]), and does
//!   not move its cursor past them, so the next read sees the line whole. A
//!   sealed segment is never written again, so its unterminated last line, and
//!   any line in the middle that is not a record, is damage, counted
//!   ([`Read::damaged`]). A line a newer build wrote is kept and counted
//!   ([`Read::unknown`]).
//! - **Readers take no lock while they read.** Under the writer's lock a read
//!   lists the segments and opens the live file, then reads unlocked. A rename
//!   never moves an open file, and a sealed segment is never renamed again, so
//!   a rotation during a read changes nothing the read sees.
//! - **The writer opens late.** A book whose live file cannot be read is
//!   opened anyway; each append tries again, and fails with the OS's reason
//!   until it can (SDD-RUST §10: only the committer's refusal stops a boot).
//!
//! Only `degradation` lines can be written until each kind's phase defines its
//! events (`eph-schema`'s `LogKind::events`).

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use eph_fs::append::{
    AppendError as WriteError, AppendLog, ROTATE_AT_BYTES, Rotation, segment_first_seq,
};
use eph_schema::document::Versioned;
use eph_schema::error::SchemaError;
use eph_schema::log::{LineRead, LogKind, LogLine, read_line, render_line};
use eph_schema::refs::Ref;
use eph_schema::time::Timestamp;

/// The live file, at the Agora's root.
pub const LOG_FILE: &str = "log.jsonl";

/// The directory sealed segments are renamed into, beside the live file.
pub const ARCHIVE_DIR: &str = "log-archive";

/// A line before the book stamps it: everything but `seq` and `ts`, which the
/// writer alone chooses, so a caller cannot collide with or reorder history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// What it is about.
    pub kind: LogKind,
    /// What happened, from the kind's closed list.
    pub event: String,
    /// Who or what it concerns, if not the harness.
    pub source: Option<String>,
    /// What it points at.
    pub refs: Vec<Ref>,
    /// The event's payload.
    pub data: serde_json::Map<String, serde_json::Value>,
}

/// One append's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appended {
    /// The line as written.
    pub line: LogLine,
    /// The rotation checked before it. A skipped one is the caller's to
    /// report (invariant 7).
    pub rotation: Rotation,
}

/// Why an append wrote nothing, or wrote and was not acknowledged.
#[derive(Debug)]
pub enum AppendError {
    /// The line breaks a rule of its format, so nothing was written.
    Invalid(SchemaError),
    /// The book could not be opened or written, so nothing was written.
    Io(io::Error),
    /// A fault point stopped the append, as the harness dying there would.
    /// At [`AppendPoint::AfterWrite`] the line is on disk and was never
    /// published.
    Fault(String),
}

impl fmt::Display for AppendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppendError::Invalid(error) => write!(f, "the line was refused: {error}"),
            AppendError::Io(error) => write!(f, "the book of record could not be written: {error}"),
            AppendError::Fault(reason) => write!(f, "the append was stopped: {reason}"),
        }
    }
}

impl std::error::Error for AppendError {}

/// Where a test may stop an append, as the harness dying there would. The seam
/// is in the production path, as the committer's is (TEST-STRATEGY §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendPoint {
    /// Before anything is written.
    BeforeWrite,
    /// After the line is synced, before it is published or acknowledged.
    AfterWrite,
}

/// Where a cursor points: a byte offset into the segments and the live file
/// read as one stream (see the module contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Cursor(u64);

impl Cursor {
    /// The book's first byte.
    pub const START: Cursor = Cursor(0);

    /// The offset, for a caller that stores it.
    pub fn offset(self) -> u64 {
        self.0
    }

    /// The cursor at `offset`, as a caller that stored one reads it back.
    pub fn at(offset: u64) -> Cursor {
        Cursor(offset)
    }
}

/// What one read found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Read {
    /// The records this build knows, in file order.
    pub entries: Vec<LogLine>,
    /// Lines a newer build wrote, kept on disk and counted.
    pub unknown: usize,
    /// Lines that are not records: damage, counted.
    pub damaged: usize,
    /// Whether the live file ended mid-line: skipped, not consumed.
    pub torn_tail: bool,
    /// Where the next read starts: after the last whole line read.
    pub cursor: Cursor,
}

type FaultHook = dyn Fn(AppendPoint) -> Result<(), String> + Send + Sync;

/// The book of record. See the module contract.
pub struct Book {
    live: PathBuf,
    archive: PathBuf,
    rotate_at: u64,
    writer: Mutex<Writer>,
    faults: Option<Arc<FaultHook>>,
}

impl fmt::Debug for Book {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Book")
            .field("live", &self.live)
            .field("rotate_at", &self.rotate_at)
            .finish_non_exhaustive()
    }
}

/// What the lock guards: the writer, once it opens, and the subscribers.
struct Writer {
    log: Option<AppendLog>,
    subscribers: Vec<Sender<LogLine>>,
    appends: u64,
}

/// The sources a read walks, listed under the lock.
struct Snapshot {
    /// Each sealed segment, oldest first, with its length.
    segments: Vec<(PathBuf, u64)>,
    /// The live file, opened under the lock, or `None` when there is none.
    live: Option<File>,
}

impl Book {
    /// The book at the Agora's `root`, rotating at [`ROTATE_AT_BYTES`]. Writes
    /// nothing; the writer opens now if it can, and at each append if not.
    pub fn open(root: &Path) -> Book {
        Book::open_rotating_at(root, ROTATE_AT_BYTES)
    }

    /// [`Book::open`] with another rotation threshold, so a test can cross a
    /// rotation without writing four mebibytes.
    pub fn open_rotating_at(root: &Path, rotate_at: u64) -> Book {
        let live = root.join(LOG_FILE);
        let archive = root.join(ARCHIVE_DIR);
        let log = AppendLog::open(&live, &archive, rotate_at, seq_of).ok();
        Book {
            live,
            archive,
            rotate_at,
            writer: Mutex::new(Writer {
                log,
                subscribers: Vec::new(),
                appends: 0,
            }),
            faults: None,
        }
    }

    /// The same book stopping its appends at the points `hook` refuses.
    pub fn with_faults(
        mut self,
        hook: impl Fn(AppendPoint) -> Result<(), String> + Send + Sync + 'static,
    ) -> Book {
        self.faults = Some(Arc::new(hook));
        self
    }

    /// Opens the writer if it is not open, and says why it cannot be.
    pub fn writable(&self) -> io::Result<()> {
        let mut writer = guard(&self.writer);
        self.writer_of(&mut writer).map(|_| ())
    }

    /// How many lines this handle has appended.
    pub fn appends(&self) -> u64 {
        guard(&self.writer).appends
    }

    /// Appends `draft` as the next line. See the module contract.
    pub fn append(&self, draft: Draft) -> Result<Appended, AppendError> {
        let mut writer = guard(&self.writer);
        self.check(AppendPoint::BeforeWrite)?;
        let log = self.writer_of(&mut writer).map_err(AppendError::Io)?;
        // Stamped in place: the writer chooses `seq` as it renders.
        let mut line = LogLine {
            schema_version: LogLine::VERSION,
            seq: 0,
            ts: Timestamp::now(),
            kind: draft.kind,
            event: draft.event,
            source: draft.source,
            refs: draft.refs,
            data: draft.data,
        };
        let appended = log
            .append(|seq| {
                line.seq = seq;
                render_line(&line)
            })
            .map_err(|error| match error {
                WriteError::Render(error) => AppendError::Invalid(error),
                // serde_json escapes every line break, so a rendered line is
                // one line; `AppendLog` checks it all the same.
                WriteError::Multiline => AppendError::Invalid(multiline()),
                WriteError::Io(error) => AppendError::Io(error),
            })?;
        writer.appends += 1;
        self.check(AppendPoint::AfterWrite)?;
        writer
            .subscribers
            .retain(|subscriber| subscriber.send(line.clone()).is_ok());
        Ok(Appended {
            line,
            rotation: appended.rotation,
        })
    }

    /// A channel that receives every line appended from now on, in file order.
    /// Dropping the receiver unsubscribes it.
    pub fn subscribe(&self) -> Receiver<LogLine> {
        let (sender, receiver) = mpsc::channel();
        guard(&self.writer).subscribers.push(sender);
        receiver
    }

    /// Every line after `cursor`, to the book's end. See the module contract.
    pub fn since(&self, cursor: Cursor) -> io::Result<Read> {
        let snapshot = self.snapshot()?;
        let mut read = Read {
            cursor,
            ..Read::default()
        };
        let mut base = 0u64;
        for (path, len) in &snapshot.segments {
            let end = base + len;
            // A segment the cursor has passed is not opened (the TypeScript
            // build's "segment skipping, with no I/O").
            if end > read.cursor.0 {
                // A sealed segment is consumed whole, its unterminated last line
                // counted as damage, so the next segment starts where this one
                // ended. No writer adds to a sealed segment; one grown by hand
                // is read as it is when it is read.
                let mut file = File::open(path)?;
                let bytes = read_from(&mut file, read.cursor.0.saturating_sub(base))?;
                consume(&bytes, false, &mut read);
                read.cursor = Cursor(end);
            }
            base = end;
        }
        if let Some(mut live) = snapshot.live {
            let skip = read.cursor.0.saturating_sub(base);
            let bytes = read_from(&mut live, skip)?;
            read.cursor = Cursor(base + skip + consume(&bytes, true, &mut read));
        }
        Ok(read)
    }

    /// The last `n` records, oldest first, reading back from the live file only
    /// as far as it must. The counts cover every line it read, and its cursor
    /// is where [`Book::since`] would end.
    pub fn tail(&self, n: usize) -> io::Result<Read> {
        let snapshot = self.snapshot()?;
        let total: u64 = snapshot.segments.iter().map(|(_, len)| len).sum();
        let mut read = Read::default();
        let mut collected: Vec<Vec<LogLine>> = Vec::new();
        let mut have = 0;
        if let Some(mut live) = snapshot.live {
            let bytes = read_from(&mut live, 0)?;
            let mut part = Read::default();
            read.cursor = Cursor(total + consume(&bytes, true, &mut part));
            merge(&mut read, &mut part, &mut collected, &mut have);
        } else {
            read.cursor = Cursor(total);
        }
        for (path, _) in snapshot.segments.iter().rev() {
            if have >= n {
                break;
            }
            let bytes = read_from(&mut File::open(path)?, 0)?;
            let mut part = Read::default();
            consume(&bytes, false, &mut part);
            merge(&mut read, &mut part, &mut collected, &mut have);
        }
        let mut entries: Vec<LogLine> = collected.into_iter().rev().flatten().collect();
        let surplus = entries.len().saturating_sub(n);
        entries.drain(..surplus);
        read.entries = entries;
        Ok(read)
    }

    /// Whether the live file ends mid-line: the last run was killed during an
    /// append, or one is being written now. An absent or empty file does not.
    pub fn ends_mid_line(&self) -> io::Result<bool> {
        let mut file = match File::open(&self.live) {
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

    /// The book's size on disk, segments and live file: what a read from
    /// [`Cursor::START`] parses.
    pub fn size_bytes(&self) -> io::Result<u64> {
        let snapshot = self.snapshot()?;
        let live = match &snapshot.live {
            Some(file) => file.metadata()?.len(),
            None => 0,
        };
        Ok(snapshot.segments.iter().map(|(_, len)| len).sum::<u64>() + live)
    }

    /// The segments, oldest first, and the live file opened, all under the
    /// writer's lock, so no rotation falls between the two.
    fn snapshot(&self) -> io::Result<Snapshot> {
        let _writer = guard(&self.writer);
        let mut segments = Vec::new();
        match fs::read_dir(&self.archive) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    if let Some(first) = segment_first_seq(&entry.file_name().to_string_lossy()) {
                        segments.push((first, entry.path(), entry.metadata()?.len()));
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        segments.sort_by_key(|(first, _, _)| *first);
        let live = match File::open(&self.live) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok(Snapshot {
            segments: segments
                .into_iter()
                .map(|(_, path, len)| (path, len))
                .collect(),
            live,
        })
    }

    fn writer_of<'w>(&self, writer: &'w mut Writer) -> io::Result<&'w mut AppendLog> {
        if writer.log.is_none() {
            writer.log = Some(AppendLog::open(
                &self.live,
                &self.archive,
                self.rotate_at,
                seq_of,
            )?);
        }
        writer
            .log
            .as_mut()
            .ok_or_else(|| io::Error::other("the book's writer did not open"))
    }

    fn check(&self, point: AppendPoint) -> Result<(), AppendError> {
        match &self.faults {
            Some(hook) => hook(point).map_err(AppendError::Fault),
            None => Ok(()),
        }
    }
}

/// The `seq` of one line, for `AppendLog`'s recovery: any line that carries one,
/// a newer build's included, since numbering must not repeat it.
fn seq_of(line: &str) -> Option<u64> {
    match read_line(line) {
        LineRead::Entry(entry) => Some(entry.seq),
        LineRead::Unknown { seq, .. } => seq,
        LineRead::Unreadable { .. } => None,
    }
}

/// The refusal of a rendered line holding a line break.
fn multiline() -> SchemaError {
    SchemaError::invalid(LogLine::FILE, "line", "renders on one line")
}

/// Reads `file` from byte `offset` to its end.
fn read_from(file: &mut File, offset: u64) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Reads every line of `bytes` into `read`, and says how many bytes it
/// consumed. An unterminated last line of the live file is skipped and not
/// consumed; anywhere else it is damage.
fn consume(bytes: &[u8], live: bool, read: &mut Read) -> u64 {
    let mut consumed = 0;
    for piece in bytes.split_inclusive(|byte| *byte == b'\n') {
        let line = match piece.strip_suffix(b"\n") {
            Some(line) => line,
            None if live => {
                read.torn_tail = true;
                break;
            }
            None => piece,
        };
        match read_line(&String::from_utf8_lossy(line)) {
            LineRead::Entry(entry) => read.entries.push(entry),
            LineRead::Unknown { .. } => read.unknown += 1,
            LineRead::Unreadable { .. } => read.damaged += 1,
        }
        consumed += piece.len() as u64;
    }
    consumed
}

/// Adds one source's counts and records to a tail read.
fn merge(read: &mut Read, part: &mut Read, collected: &mut Vec<Vec<LogLine>>, have: &mut usize) {
    read.unknown += part.unknown;
    read.damaged += part.damaged;
    read.torn_tail |= part.torn_tail;
    *have += part.entries.len();
    collected.push(std::mem::take(&mut part.entries));
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use eph_fs::append::segment_name;
    use eph_fs::atomic::write_atomic;
    use eph_schema::degradation::{Cause, Degradation};

    use super::*;
    use crate::agora::book_conditions;
    use crate::degradations::Degradations;

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("eph-agora-book-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A degradation line's draft, told apart by its detail.
    pub(crate) fn draft(n: u64) -> Draft {
        let row = Degradation {
            cause: "agora/test:subject".parse::<Cause>().unwrap(),
            detail: format!("line {n}"),
            count: n.max(1),
            since: Timestamp::from_millis(1_000).unwrap(),
            cleared_after: None,
        };
        let line = row
            .to_line(1, Timestamp::from_millis(1_000).unwrap())
            .unwrap();
        Draft {
            kind: line.kind,
            event: line.event,
            source: line.source,
            refs: line.refs,
            data: line.data,
        }
    }

    fn details(read: &Read) -> Vec<String> {
        read.entries
            .iter()
            .map(|line| line.data["detail"].as_str().unwrap().to_string())
            .collect()
    }

    /// The bytes one append of `draft(n)` at `seq` writes, newline included.
    fn bytes_of(n: u64, seq: u64) -> Vec<u8> {
        let draft = draft(n);
        let line = LogLine {
            schema_version: LogLine::VERSION,
            seq,
            ts: Timestamp::from_millis(2_000).unwrap(),
            kind: draft.kind,
            event: draft.event,
            source: draft.source,
            refs: draft.refs,
            data: draft.data,
        };
        let mut text = render_line(&line).unwrap().into_bytes();
        text.push(b'\n');
        text
    }

    fn live_len(dir: &Path) -> u64 {
        fs::metadata(dir.join(LOG_FILE)).unwrap().len()
    }

    #[test]
    fn appends_are_stamped_in_order_and_read_back_from_a_cursor() {
        let dir = scratch("order");
        let book = Book::open(&dir);
        for n in 1..=3 {
            let appended = book.append(draft(n)).unwrap();
            assert_eq!(appended.line.seq, n);
            assert_eq!(appended.rotation, Rotation::NotDue);
        }
        assert_eq!(book.appends(), 3);
        let read = book.since(Cursor::START).unwrap();
        assert_eq!(details(&read), ["line 1", "line 2", "line 3"]);
        assert_eq!(
            read.entries.iter().map(|line| line.seq).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(read.cursor.offset(), live_len(&dir));
        assert_eq!((read.unknown, read.damaged, read.torn_tail), (0, 0, false));
        let after = book.since(read.cursor).unwrap();
        assert!(after.entries.is_empty());
        assert_eq!(after.cursor, read.cursor, "a read at the end stays there");
        book.append(draft(4)).unwrap();
        assert_eq!(details(&book.since(read.cursor).unwrap()), ["line 4"]);
        assert_eq!(Cursor::at(read.cursor.offset()), read.cursor);
        let reopened = Book::open(&dir);
        assert_eq!(
            reopened.append(draft(5)).unwrap().line.seq,
            5,
            "seq is recovered"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Two appenders at once, one live subscriber and one that left. That a
    /// line is published only after it is on disk is the AfterWrite fault
    /// test's: a line stopped there is on disk and never published.
    #[test]
    fn every_live_subscriber_gets_every_line_in_file_order_and_a_dropped_one_leaves() {
        let dir = scratch("publish");
        let book = Arc::new(Book::open(&dir));
        let kept = book.subscribe();
        let gone = book.subscribe();
        drop(gone);
        let appenders: Vec<_> = (0..2)
            .map(|_| {
                let book = Arc::clone(&book);
                thread::spawn(move || {
                    for n in 1..=50 {
                        book.append(draft(n)).unwrap();
                    }
                })
            })
            .collect();
        for appender in appenders {
            appender.join().unwrap();
        }
        let heard: Vec<u64> = kept.try_iter().map(|line| line.seq).collect();
        let on_disk: Vec<u64> = book
            .since(Cursor::START)
            .unwrap()
            .entries
            .iter()
            .map(|line| line.seq)
            .collect();
        assert_eq!(
            heard,
            (1..=100).collect::<Vec<_>>(),
            "no seq repeated or skipped"
        );
        assert_eq!(heard, on_disk, "the order heard is the file's order");
        assert_eq!(
            guard(&book.writer).subscribers.len(),
            1,
            "the dropped receiver unsubscribed itself; the live one stayed"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rotation_during_reads_changes_nothing_a_reader_sees() {
        let dir = scratch("rotate-reading");
        let one = bytes_of(1, 1).len() as u64;
        let book = Arc::new(Book::open_rotating_at(&dir, one * 3));
        let writer = {
            let book = Arc::clone(&book);
            thread::spawn(move || {
                for n in 1..=200 {
                    book.append(draft(n)).unwrap();
                }
            })
        };
        let mut cursor = Cursor::START;
        let mut seen = Vec::new();
        let mut damaged = 0;
        loop {
            let done = writer.is_finished();
            let read = book.since(cursor).unwrap();
            damaged += read.damaged;
            seen.extend(read.entries.iter().map(|line| line.seq));
            cursor = read.cursor;
            if done {
                break;
            }
        }
        writer.join().unwrap();
        assert_eq!(
            seen,
            (1..=200).collect::<Vec<_>>(),
            "no line lost, repeated or reordered"
        );
        assert_eq!(damaged, 0);
        assert_eq!(cursor.offset(), book.size_bytes().unwrap());
        assert!(
            fs::read_dir(dir.join(ARCHIVE_DIR)).unwrap().count() > 10,
            "the reads crossed many rotations"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_cursor_taken_before_a_rotation_points_at_the_same_line_after_it() {
        let dir = scratch("rotate");
        let one = bytes_of(1, 1).len() as u64;
        let book = Book::open_rotating_at(&dir, one * 2);
        book.append(draft(1)).unwrap();
        book.append(draft(2)).unwrap();
        let cursor = book.since(Cursor::START).unwrap().cursor;
        let third = book.append(draft(3)).unwrap();
        assert!(
            matches!(third.rotation, Rotation::Sealed(_)),
            "{:?}",
            third.rotation
        );
        for n in 4..=6 {
            book.append(draft(n)).unwrap();
        }
        assert!(dir.join(ARCHIVE_DIR).join(segment_name(1)).is_file());
        assert!(dir.join(ARCHIVE_DIR).join(segment_name(3)).is_file());
        let after = book.since(cursor).unwrap();
        assert_eq!(details(&after), ["line 3", "line 4", "line 5", "line 6"]);
        let middle = book.since(Cursor::at(cursor.offset() + one)).unwrap();
        assert_eq!(
            details(&middle),
            ["line 4", "line 5", "line 6"],
            "a cursor inside a sealed segment reads from there"
        );
        let whole = book.since(Cursor::START).unwrap();
        assert_eq!(
            whole.entries.len(),
            6,
            "no reader loses history to a rotation"
        );
        assert_eq!(whole.cursor, after.cursor);
        assert_eq!(whole.cursor.offset(), book.size_bytes().unwrap());
        let tail = book.tail(4).unwrap();
        assert_eq!(details(&tail), ["line 3", "line 4", "line 5", "line 6"]);
        assert_eq!(
            tail.cursor, whole.cursor,
            "a tail's cursor continues where it ends"
        );
        assert_eq!(details(&book.tail(1).unwrap()), ["line 6"]);
        assert_eq!(book.tail(99).unwrap().entries.len(), 6);
        assert!(book.tail(0).unwrap().entries.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_live_tail_is_skipped_unconsumed_and_read_whole_once_finished() {
        let dir = scratch("torn");
        let book = Book::open(&dir);
        book.append(draft(1)).unwrap();
        let line = bytes_of(2, 2);
        let mut bytes = fs::read(dir.join(LOG_FILE)).unwrap();
        let cut = line.len() / 2;
        bytes.extend_from_slice(&line[..cut]);
        write_atomic(&dir.join(LOG_FILE), &bytes).unwrap();
        let read = book.since(Cursor::START).unwrap();
        assert!(read.torn_tail);
        assert_eq!(details(&read), ["line 1"]);
        assert_eq!(read.damaged, 0, "a torn last line is not damage");
        assert!(book.ends_mid_line().unwrap());
        let tail = book.tail(5).unwrap();
        assert!(tail.torn_tail, "a tail says so too");
        assert_eq!(tail.cursor, read.cursor);
        // The writer finishes the line: the cursor stopped before it.
        bytes.extend_from_slice(&line[cut..]);
        write_atomic(&dir.join(LOG_FILE), &bytes).unwrap();
        let finished = book.since(read.cursor).unwrap();
        assert_eq!(details(&finished), ["line 2"]);
        assert!(!finished.torn_tail);
        assert!(!book.ends_mid_line().unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn damage_in_a_segment_or_the_middle_is_counted_and_newer_lines_kept() {
        let dir = scratch("damage");
        let archive = dir.join(ARCHIVE_DIR);
        fs::create_dir_all(&archive).unwrap();
        let mut segment = bytes_of(1, 1);
        segment.extend_from_slice(b"not a record\n");
        segment.extend_from_slice(&bytes_of(2, 2)[..20]);
        write_atomic(&archive.join(segment_name(1)), &segment).unwrap();
        let newer = String::from_utf8(bytes_of(3, 7))
            .unwrap()
            .replace("\"schemaVersion\":1", "\"schemaVersion\":9");
        let mut live = newer.into_bytes();
        live.extend_from_slice(b"\n");
        live.extend_from_slice(&bytes_of(4, 4));
        write_atomic(&dir.join(LOG_FILE), &live).unwrap();
        write_atomic(&archive.join("notes.txt"), b"not a segment\n").unwrap();
        let book = Book::open(&dir);
        let read = book.since(Cursor::START).unwrap();
        assert_eq!(details(&read), ["line 1", "line 4"]);
        assert_eq!(read.unknown, 1, "a newer build's line is kept and counted");
        assert_eq!(
            read.damaged, 3,
            "a line that is not a record, a sealed segment's torn end, and a blank line"
        );
        assert!(!read.torn_tail, "only the live file has a torn tail");
        let tail = book.tail(1).unwrap();
        assert_eq!(details(&tail), ["line 4"]);
        assert_eq!(
            (tail.unknown, tail.damaged),
            (1, 1),
            "the live file alone was read"
        );
        assert_eq!(
            book.append(draft(5)).unwrap().line.seq,
            8,
            "numbering passes the seq a newer build wrote"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_book_that_cannot_be_written_opens_anyway_and_heals_when_it_can() {
        let dir = scratch("unwritable");
        fs::create_dir_all(dir.join(LOG_FILE).join("inside")).unwrap();
        let book = Book::open(&dir);
        assert!(book.writable().is_err());
        assert!(matches!(book.append(draft(1)), Err(AppendError::Io(_))));
        assert!(
            book.since(Cursor::START).is_err(),
            "nor is it read as empty"
        );
        assert_eq!(book.appends(), 0);
        fs::remove_dir_all(dir.join(LOG_FILE)).unwrap();
        assert!(book.writable().is_ok());
        assert_eq!(book.append(draft(1)).unwrap().line.seq, 1);
        let error = AppendError::Io(io::Error::other("full")).to_string();
        assert!(error.contains("could not be written: full"), "{error}");
        // A live file that goes bad after the writer opened fails the append.
        fs::remove_file(dir.join(LOG_FILE)).unwrap();
        fs::create_dir_all(dir.join(LOG_FILE)).unwrap();
        assert!(matches!(book.append(draft(2)), Err(AppendError::Io(_))));
        assert_eq!(book.appends(), 1, "the refused line was not counted");
        assert!(
            format!("{book:?}").contains("rotate_at: 4194304"),
            "{book:?}"
        );
        let refused = AppendError::Invalid(multiline()).to_string();
        assert!(refused.contains("renders on one line"), "{refused}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_book_s_size_counts_its_segments_and_an_archive_that_is_a_file_is_an_error() {
        let dir = scratch("size");
        let book = Book::open(&dir);
        assert_eq!(book.size_bytes().unwrap(), 0, "no segment and no live file");
        let archive = dir.join(ARCHIVE_DIR);
        fs::create_dir_all(&archive).unwrap();
        write_atomic(&archive.join(segment_name(1)), &bytes_of(1, 1)).unwrap();
        assert_eq!(
            book.size_bytes().unwrap(),
            bytes_of(1, 1).len() as u64,
            "a sealed segment with no live file after it"
        );
        assert_eq!(details(&book.since(Cursor::START).unwrap()), ["line 1"]);
        fs::remove_dir_all(&archive).unwrap();
        write_atomic(&archive, b"not a directory").unwrap();
        assert!(book.since(Cursor::START).is_err(), "never read as empty");
        assert!(book.tail(1).is_err());
        assert!(book.size_bytes().is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fault_stops_an_append_before_its_write_or_after_it_unpublished() {
        for point in [AppendPoint::BeforeWrite, AppendPoint::AfterWrite] {
            let dir = scratch(&format!("fault-{point:?}"));
            let book = Book::open(&dir).with_faults(move |at| {
                if at == point {
                    return Err("the harness died".to_string());
                }
                Ok(())
            });
            let heard = book.subscribe();
            let stopped = book.append(draft(1)).unwrap_err();
            assert!(
                matches!(&stopped, AppendError::Fault(why) if why == "the harness died"),
                "{stopped:?}"
            );
            assert!(stopped.to_string().contains("stopped"), "{stopped}");
            let read = book.since(Cursor::START).unwrap();
            assert_eq!(
                read.entries.len(),
                usize::from(point == AppendPoint::AfterWrite),
                "{point:?}: on disk only after its write"
            );
            assert!(heard.try_recv().is_err(), "{point:?}: never published");
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn an_invalid_line_writes_nothing_and_spends_no_seq() {
        let dir = scratch("invalid");
        let book = Book::open(&dir);
        let mut gym = draft(1);
        gym.kind = LogKind::Gym;
        let refused = book.append(gym).unwrap_err();
        assert!(matches!(refused, AppendError::Invalid(_)), "{refused:?}");
        assert!(refused.to_string().contains("not defined yet"), "{refused}");
        assert!(!dir.join(LOG_FILE).exists());
        assert_eq!(book.append(draft(1)).unwrap().line.seq, 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn whether_the_live_file_ends_mid_line() {
        let dir = scratch("mid-line");
        let book = Book::open(&dir);
        assert!(!book.ends_mid_line().unwrap(), "absent");
        write_atomic(&dir.join(LOG_FILE), b"").unwrap();
        assert!(!book.ends_mid_line().unwrap(), "empty");
        write_atomic(&dir.join(LOG_FILE), b"{}\n").unwrap();
        assert!(!book.ends_mid_line().unwrap());
        write_atomic(&dir.join(LOG_FILE), b"{}\n{").unwrap();
        assert!(book.ends_mid_line().unwrap());
        fs::remove_file(dir.join(LOG_FILE)).unwrap();
        fs::create_dir_all(dir.join(LOG_FILE)).unwrap();
        assert!(
            book.ends_mid_line().is_err(),
            "a directory is not an empty book"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// An append is one synced `write_all` of `[\n]line\n`, so a kill leaves a
    /// prefix of those bytes. Every prefix, after real earlier lines, and after
    /// an earlier torn line (DECISIONS-LOG 2026-10-25, *R1.4b — Architect
    /// decisions, second round*, item 7).
    #[test]
    fn every_prefix_a_killed_append_can_leave_loses_and_tears_nothing_before_it() {
        let dir = scratch("prefixes");
        let earlier = [bytes_of(1, 1), bytes_of(2, 2)].concat();
        let third = bytes_of(3, 3);
        let object = third.len() - 1;
        for torn_before in [false, true] {
            let mut before = earlier.clone();
            let lead = usize::from(torn_before);
            if torn_before {
                before.extend_from_slice(&bytes_of(9, 9)[..15]);
            }
            let mut append = vec![b'\n'; lead];
            append.extend_from_slice(&third);
            for k in 0..=append.len() {
                let what = format!("k={k} torn_before={torn_before}");
                let mut file = before.clone();
                file.extend_from_slice(&append[..k]);
                write_atomic(&dir.join(LOG_FILE), &file).unwrap();
                // Bytes of the third line written, and whether its object is
                // whole: the writer then recovers its seq, though the line
                // has no newline yet.
                let written = k.saturating_sub(lead);
                let object_whole = written >= object;
                let book = Book::open(&dir);
                let read = book.since(Cursor::START).unwrap();
                let mut expected = vec!["line 1", "line 2"];
                if written == third.len() {
                    expected.push("line 3");
                }
                assert_eq!(details(&read), expected, "{what}");
                let tail_torn = file.last() != Some(&b'\n');
                assert_eq!(read.torn_tail, tail_torn, "{what}");
                assert_eq!(book.ends_mid_line().unwrap(), tail_torn, "{what}");
                let next = book.append(draft(4)).unwrap();
                assert_eq!(next.line.seq, if object_whole { 4 } else { 3 }, "{what}");
                let after = book.since(Cursor::START).unwrap();
                let mut expected = vec!["line 1", "line 2"];
                if object_whole {
                    expected.push("line 3");
                }
                expected.push("line 4");
                assert_eq!(details(&after), expected, "{what}");
                assert!(!after.torn_tail, "{what}: the next line starts fresh");
                let damaged = lead + usize::from(written > 0 && !object_whole);
                assert_eq!(after.damaged, damaged, "{what}");
            }
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    // The kill tests. Each starts this test binary again as a writer that
    // appends in a loop and prints each `seq` it was told about, and kills it:
    // by its own exit at a fault point, or by the OS after a fixed delay that
    // may or may not land inside an append (GYM-014). They assert the outcome
    // on reopen, never where the kill landed. On Windows a kill rarely lands
    // inside one small write, so the state they add over the planted prefixes
    // is a line on disk that was never acknowledged.

    /// The variable naming the directory a child writes its book in.
    const CHILD_BOOK: &str = "EPH_AGORA_TEST_BOOK";
    /// The variable naming what a child does.
    const CHILD_MODE: &str = "EPH_AGORA_TEST_MODE";
    /// The exit code of a child that died at its fault point.
    const DIED: i32 = 86;
    /// The longest a test waits on a child, inside a mutation round's
    /// per-mutant timeout, so a mutant that hangs a child fails.
    const CHILD_PATIENCE: Duration = Duration::from_secs(10);

    #[expect(
        clippy::disallowed_methods,
        reason = "process: the kill tests run this test binary again, as the writer they kill (GYM-014)"
    )]
    fn spawn_child(dir: &Path, mode: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "book::tests::child",
                "--test-threads=1",
                "--nocapture",
                "--quiet",
            ])
            .env(CHILD_BOOK, dir)
            .env(CHILD_MODE, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Each `seq` a child prints as acknowledged, as it arrives, read on a
    /// thread so a full pipe never stalls the child.
    fn acknowledged(child: &mut Child) -> (mpsc::Receiver<u64>, thread::JoinHandle<()>) {
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (sender, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if let Some(seq) = line
                    .strip_prefix("ack:")
                    .and_then(|seq| seq.trim().parse().ok())
                {
                    let _sent = sender.send(seq);
                }
            }
        });
        (receiver, reader)
    }

    /// Waits for `child` to exit, killing it if it outlives [`CHILD_PATIENCE`].
    fn exit_of(child: &mut Child) -> Option<i32> {
        let deadline = Instant::now() + CHILD_PATIENCE;
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status.code();
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let _reaped = child.wait();
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Not a test of its own: the body of the child the kill tests start. Run
    /// without their variables, it does nothing.
    #[test]
    fn child() {
        let (Some(dir), Ok(mode)) = (std::env::var_os(CHILD_BOOK), std::env::var(CHILD_MODE))
        else {
            return;
        };
        let point = match mode.as_str() {
            "before-write" => Some(AppendPoint::BeforeWrite),
            "after-write" => Some(AppendPoint::AfterWrite),
            _ => None,
        };
        let book = Book::open(Path::new(&dir)).with_faults(move |at| {
            if Some(at) == point {
                std::process::exit(DIED);
            }
            Ok(())
        });
        for n in 1.. {
            let appended = book.append(draft(n)).unwrap();
            println!("ack:{}", appended.line.seq);
        }
    }

    /// After a kill: every acknowledged line reads back, nothing before the
    /// last line is torn, the reopen raises `agora/log-torn` exactly when the
    /// live file ends mid-line (GYM-014), and the next append is whole.
    fn assert_nothing_lost(dir: &Path, acked: &[u64], what: &str) {
        let book = Arc::new(Book::open(dir));
        let channel = Degradations::new(Arc::clone(&book));
        let conditions: Vec<String> = book_conditions(&book, &channel)
            .iter()
            .map(|condition| condition.cause.to_string())
            .collect();
        let read = book.since(Cursor::START).unwrap();
        let torn = if read.torn_tail {
            vec!["agora/log-torn".to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(conditions, torn, "{what}: the reopen's own report");
        let seqs: Vec<u64> = read.entries.iter().map(|line| line.seq).collect();
        for seq in acked {
            assert!(
                seqs.contains(seq),
                "{what}: acknowledged seq {seq} was lost: {seqs:?}"
            );
        }
        assert_eq!(
            read.damaged, 0,
            "{what}: nothing before the last line is torn"
        );
        assert_eq!(book.ends_mid_line().unwrap(), read.torn_tail, "{what}");
        let next = book.append(draft(0)).unwrap();
        let after = book.since(Cursor::START).unwrap();
        assert_eq!(after.entries.last().unwrap().seq, next.line.seq, "{what}");
        assert!(after.entries.len() > read.entries.len(), "{what}");
        assert!(
            after.damaged <= usize::from(read.torn_tail),
            "{what}: at most the torn line became damage"
        );
    }

    #[test]
    fn a_writer_killed_at_each_point_of_an_append_loses_nothing_acknowledged() {
        for (mode, on_disk) in [("before-write", 0), ("after-write", 1)] {
            let dir = scratch(&format!("killed-{mode}"));
            let mut child = spawn_child(&dir, mode);
            let (acks, reader) = acknowledged(&mut child);
            assert_eq!(
                exit_of(&mut child),
                Some(DIED),
                "{mode}: the child died at its point"
            );
            reader.join().unwrap();
            let acked: Vec<u64> = acks.try_iter().collect();
            assert!(
                acked.is_empty(),
                "{mode}: it died before its first acknowledgement"
            );
            let read = Book::open(&dir).since(Cursor::START).unwrap();
            assert_eq!(
                read.entries.len(),
                on_disk,
                "{mode}: a line written and never acknowledged is still on disk"
            );
            assert_nothing_lost(&dir, &acked, mode);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_writer_killed_at_random_loses_nothing_it_acknowledged() {
        for after in [0, 40, 90, 160, 250] {
            let dir = scratch(&format!("killed-random-{after}"));
            let mut child = spawn_child(&dir, "churn");
            let (acks, reader) = acknowledged(&mut child);
            // The delay starts at the first acknowledgement, so a child slow
            // to start (an instrumented build, a scanner) is still killed
            // while it writes.
            let first = acks.recv_timeout(CHILD_PATIENCE);
            thread::sleep(Duration::from_millis(after));
            child.kill().unwrap();
            let _reaped = child.wait().unwrap();
            reader.join().unwrap();
            let first = first.expect("the child acknowledged a line before its kill");
            let acked: Vec<u64> = std::iter::once(first).chain(acks.try_iter()).collect();
            assert_nothing_lost(
                &dir,
                &acked,
                &format!("killed {after} ms after its first line"),
            );
            fs::remove_dir_all(&dir).unwrap();
        }
    }
}

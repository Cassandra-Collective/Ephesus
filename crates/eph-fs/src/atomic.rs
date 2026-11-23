//! `write_atomic`: the one write path for a file another process reads
//! (SDD-RUST §4.0 rule 5; invariant 3).
//!
//! Contract: the file at `path` holds either its old bytes or all of the new
//! ones, never a part. The bytes go to a temporary file beside it
//! (`.<name>.<8 hex>.tmp`), which is flushed and synced to disk, then renamed
//! over the target. On POSIX the directory is synced too, so the rename itself
//! survives a power cut. A failed write removes its temporary file and leaves
//! the target untouched.
//!
//! **Windows can refuse a rename for a moment**, for reasons outside this
//! program: a virus scanner or search indexer holding one of the two files.
//! Measured on Windows 11 (R1.2, 2026-10-20):
//! - a handle on the *destination* that does not share delete access makes
//!   `rename` fail with OS error 5 (access denied), and the target keeps its
//!   old bytes;
//! - a handle on the *source*, which is what a scanner reading a freshly
//!   written temporary file is, makes it fail with OS error 32 (sharing
//!   violation);
//! - renaming over a directory also fails with error 5, and that one is
//!   permanent.
//!
//! So those two codes are retried on Windows, error 5 only when the destination
//! is not a directory, over a fixed schedule of waits ([`WAITS`]: 5 ms doubling
//! to 100 ms) that sums to today's budget of 500 ms. A delivery's p95 is 500 ms
//! (NFR-2), and a transient hold is far rarer than one delivery in twenty. The
//! schedule is finite by construction, so no change to a condition can make the
//! retry loop forever (`eph-fs` mutation round 1 found three that did, against
//! a loop bounded by a clock, 2026-10-20). Nothing is retried on any other
//! platform, where neither code means a passing hold.
//! The residual: a crash between writing the temporary file and renaming it
//! leaves that `.tmp` file beside its target. Nothing sweeps it yet, because a
//! sweep is safe only once boot knows no other harness is live (ADR-0034).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a refused rename is waited on, in all, before the write fails.
pub const RENAME_BUDGET: Duration = Duration::from_millis(500);

/// The waits between attempts, in milliseconds: 5 doubling to 100, then 100
/// until the budget is spent. They sum to [`RENAME_BUDGET`], which a test pins.
pub const WAITS: [u64; 9] = [5, 10, 20, 40, 80, 100, 100, 100, 45];

/// Windows' "access denied": a held destination, or a directory there.
const ACCESS_DENIED: i32 = 5;

/// Windows' "sharing violation": a held source.
const SHARING_VIOLATION: i32 = 32;

/// Whether a refused rename is a passing hold worth waiting out. Pure: the
/// platform is a parameter, so the rule is tested on every platform.
pub fn is_transient(raw_os_error: Option<i32>, on_windows: bool, destination_is_dir: bool) -> bool {
    on_windows
        && match raw_os_error {
            Some(SHARING_VIOLATION) => true,
            Some(ACCESS_DENIED) => !destination_is_dir,
            _ => false,
        }
}

/// Runs `rename` until it succeeds, fails for good, or the [`WAITS`] are spent:
/// one attempt, then one more after each wait. `transient` judges each failure
/// and `sleep` waits; both are parameters so the loop is tested without a real
/// hold or a real clock.
pub fn retry_rename(
    mut rename: impl FnMut() -> io::Result<()>,
    transient: impl Fn(&io::Error) -> bool,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    for wait in WAITS {
        match rename() {
            Ok(()) => return Ok(()),
            Err(error) if transient(&error) => sleep(Duration::from_millis(wait)),
            Err(error) => return Err(error),
        }
    }
    rename()
}

/// The temporary file `path`'s bytes are written to first: hidden, in the same
/// directory (a rename cannot cross filesystems), and unique per write.
pub fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.tmp", eph_schema::id::random_suffix()))
}

/// Writes `bytes` to `path` atomically. See the module contract.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_path(path);
    let result = write_temp(&tmp, bytes).and_then(|()| {
        retry_rename(
            || fs::rename(&tmp, path),
            |error| is_transient(error.raw_os_error(), cfg!(windows), path.is_dir()),
            std::thread::sleep,
        )
    });
    match result {
        Ok(()) => sync_dir(path),
        Err(error) => {
            // The temporary file is ours and holds nothing anyone reads.
            let _removed = fs::remove_file(&tmp);
            Err(error)
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "fs-write: the temporary half of write_atomic, created new and never a live path"
)]
fn write_temp(tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Syncs the directory holding `path`, so the rename survives a power cut. A
/// directory cannot be opened as a file on Windows, whose NTFS journals the
/// rename itself, so this is POSIX only.
fn sync_dir(path: &Path) -> io::Result<()> {
    if cfg!(unix)
        && let Some(dir) = path.parent()
    {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-fs-atomic-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_write_replaces_the_file_whole_and_leaves_no_temporary() {
        let dir = scratch("whole");
        let target = dir.join("registry.json");
        write_atomic(&target, b"first").unwrap();
        write_atomic(&target, b"second, longer than the first").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"second, longer than the first");
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["registry.json"], "no temporary file is left behind");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_the_target_and_no_temporary() {
        let dir = scratch("failed");
        let target = dir.join("sub");
        fs::create_dir_all(target.join("inside")).unwrap();
        let error = write_atomic(&target, b"x").unwrap_err();
        assert!(
            target.join("inside").is_dir(),
            "the directory is untouched: {error}"
        );
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["sub"], "the temporary file was removed");
        let missing = dir.join("no-such-dir").join("x.json");
        assert!(
            write_atomic(&missing, b"x").is_err(),
            "a missing directory is an error"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_temporary_file_is_hidden_beside_the_target_and_unique() {
        let target = Path::new("/w/agora/registry.json");
        let a = temp_path(target);
        let b = temp_path(target);
        assert_ne!(a, b);
        assert_eq!(a.parent(), target.parent());
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(".registry.json.") && name.ends_with(".tmp"),
            "{name}"
        );
        assert_eq!(name.len(), ".registry.json.".len() + 8 + ".tmp".len());
    }

    #[test]
    fn only_the_two_measured_windows_codes_are_waited_out() {
        assert!(is_transient(Some(32), true, false), "a held source");
        assert!(is_transient(Some(5), true, false), "a held destination");
        assert!(
            !is_transient(Some(5), true, true),
            "a directory there is permanent"
        );
        assert!(
            is_transient(Some(32), true, true),
            "a held source, whatever the destination"
        );
        for raw in [None, Some(2), Some(3), Some(183)] {
            assert!(!is_transient(raw, true, false), "{raw:?}");
        }
        for raw in [Some(5), Some(32), Some(1), Some(13), Some(16)] {
            assert!(
                !is_transient(raw, false, false),
                "nothing is retried off Windows: {raw:?}"
            );
        }
    }

    fn denied() -> io::Error {
        io::Error::from_raw_os_error(5)
    }

    #[test]
    fn the_waits_double_to_a_cap_and_sum_to_the_budget() {
        assert_eq!(WAITS, [5, 10, 20, 40, 80, 100, 100, 100, 45]);
        assert_eq!(Duration::from_millis(WAITS.iter().sum()), RENAME_BUDGET);
    }

    #[test]
    fn a_passing_hold_is_waited_out_on_the_schedule() {
        let attempts = Cell::new(0);
        let waits = RefCell::new(Vec::new());
        let result = retry_rename(
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 7 {
                    Err(denied())
                } else {
                    Ok(())
                }
            },
            |_| true,
            |wait| waits.borrow_mut().push(wait.as_millis()),
        );
        assert!(result.is_ok());
        assert_eq!(attempts.get(), 7);
        assert_eq!(*waits.borrow(), [5, 10, 20, 40, 80, 100]);
    }

    #[test]
    fn a_hold_past_the_budget_and_a_permanent_failure_both_fail() {
        let attempts = Cell::new(0u32);
        let waited = Cell::new(Duration::ZERO);
        let result = retry_rename(
            || {
                attempts.set(attempts.get() + 1);
                Err(denied())
            },
            |_| true,
            |wait| waited.set(waited.get() + wait),
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(5));
        assert_eq!(
            waited.get(),
            RENAME_BUDGET,
            "exactly the whole budget was waited"
        );
        assert_eq!(
            attempts.get(),
            10,
            "one attempt, then one after each of the nine waits"
        );
        let once = Cell::new(0);
        let result = retry_rename(
            || {
                once.set(once.get() + 1);
                Err(denied())
            },
            |_| false,
            |_| panic!("a permanent failure is not waited on"),
        );
        assert!(result.is_err());
        assert_eq!(once.get(), 1);
        let late = Cell::new(0);
        let result = retry_rename(
            || {
                late.set(late.get() + 1);
                if late.get() < 10 {
                    Err(denied())
                } else {
                    Ok(())
                }
            },
            |_| true,
            |_| {},
        );
        assert!(
            result.is_ok(),
            "the attempt after the last wait still counts"
        );
    }

    /// The hold a virus scanner makes, reproduced: a handle on the destination
    /// that does not share delete access, released after 120 ms.
    #[cfg(windows)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "fs-write: the open is read-only and writes nothing; it holds the file as a scanner would"
    )]
    fn a_real_hold_on_windows_is_waited_out_and_one_past_the_budget_fails() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = scratch("held");
        let target = dir.join("tasks.json");
        write_atomic(&target, b"old").unwrap();
        let held = fs::File::options()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            drop(held);
        });
        write_atomic(&target, b"new").unwrap();
        releaser.join().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");

        let held = fs::File::options()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        let started = std::time::Instant::now();
        let error = write_atomic(&target, b"lost").unwrap_err();
        assert!(
            started.elapsed() >= RENAME_BUDGET,
            "the budget was spent: {error}"
        );
        assert_eq!(error.raw_os_error(), Some(5));
        drop(held);
        assert_eq!(
            fs::read(&target).unwrap(),
            b"new",
            "the target kept its bytes"
        );
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "the temporary file was removed"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

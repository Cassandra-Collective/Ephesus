//! `Exclusive`: a lock one holder keeps for as long as it runs. It is the
//! single committer's lock on its Agora (R1.3; DECISIONS-LOG 2026-10-20, *R1.3 —
//! Architect decisions*).
//!
//! Contract: [`Exclusive::try_acquire`] opens the file at `path` in place,
//! creating it empty when it is absent and never truncating it, and takes the
//! OS's exclusive lock on it (`File::try_lock`). It returns `Ok(None)` when
//! another handle holds that lock, in this process or in another. The lock is
//! released when the [`Exclusive`] is dropped, and by the OS when the process
//! dies, however it dies. So there is no stale lock to clean up, and the file is
//! never deleted: deleting it is what would let a second holder in.
//!
//! - **Opened in place, never through `write_atomic`.** A rename swaps the file
//!   a lock is held on, so a lock taken on a renamed-in file guards nothing.
//! - **On Windows, opened without delete sharing.** Measured (win32, 2026-10-20):
//!   with std's default share mode another process can delete a held lock file,
//!   and a new file at the same path then takes a second, independent lock.
//!   Without `FILE_SHARE_DELETE` that delete is refused. On POSIX a held file
//!   can always be unlinked. Nothing in Ephesus deletes one, and a deletion by
//!   hand while it is held is the recorded residual.
//! - The file holds no bytes, and nothing reads it.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// `FILE_SHARE_READ | FILE_SHARE_WRITE`: everything but `FILE_SHARE_DELETE`.
#[cfg(windows)]
const SHARE_READ_WRITE: u32 = 0x1 | 0x2;

/// A held exclusive lock. Dropping it releases the lock.
#[must_use = "the lock is released as soon as this is dropped"]
#[derive(Debug)]
pub struct Exclusive {
    _file: File,
    path: PathBuf,
}

impl Exclusive {
    /// Takes the lock on `path`, or returns `Ok(None)` when it is held. See the
    /// module contract.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Exclusive>> {
        let file = open_in_place(path)?;
        let locked = match file.try_lock() {
            Err(TryLockError::WouldBlock) => false,
            other => {
                other?;
                true
            }
        };
        Ok(locked.then(|| Exclusive {
            _file: file,
            path: path.to_path_buf(),
        }))
    }

    /// The lock file this lock is held on.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "fs-write: a lock file, opened in place without truncation; it holds no bytes and nothing writes to it"
)]
fn open_in_place(path: &Path) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(SHARE_READ_WRITE);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-fs-lock-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = scratch("second");
        let path = dir.join("committer.lock");
        let first = Exclusive::try_acquire(&path)
            .unwrap()
            .expect("a free lock is taken");
        assert_eq!(first.path(), path);
        assert!(
            Exclusive::try_acquire(&path).unwrap().is_none(),
            "a held lock is refused to a second handle"
        );
        drop(first);
        let second = Exclusive::try_acquire(&path).unwrap();
        assert!(second.is_some(), "a released lock is free again");
        drop(second);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_lock_file_is_made_empty_and_never_truncated() {
        let dir = scratch("bytes");
        let made = dir.join("made.lock");
        drop(Exclusive::try_acquire(&made).unwrap().unwrap());
        assert_eq!(
            fs::read(&made).unwrap(),
            b"",
            "a new lock file holds no bytes"
        );
        let kept = dir.join("kept.lock");
        crate::atomic::write_atomic(&kept, b"left by someone").unwrap();
        drop(Exclusive::try_acquire(&kept).unwrap().unwrap());
        assert_eq!(
            fs::read(&kept).unwrap(),
            b"left by someone",
            "taking a lock never rewrites the file"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_whose_directory_is_missing_is_an_error_not_a_refusal() {
        let dir = scratch("missing");
        let error = Exclusive::try_acquire(&dir.join("absent").join("committer.lock")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_held_lock_file_cannot_be_deleted_on_windows() {
        let dir = scratch("delete");
        let path = dir.join("committer.lock");
        let held = Exclusive::try_acquire(&path).unwrap().unwrap();
        let refused = fs::remove_file(&path).unwrap_err();
        assert_eq!(
            refused.raw_os_error(),
            Some(32),
            "a delete of a held lock file is a sharing violation"
        );
        assert!(path.exists());
        drop(held);
        fs::remove_file(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}

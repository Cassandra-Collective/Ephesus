//! `Store<T>`: the one store for structured records (SDD-RUST §4.0 rule 6, P5).
//!
//! Contract:
//! - **Absent is not damaged.** [`Store::load`] answers [`Loaded::Absent`] only
//!   when nothing exists at the path, [`Loaded::Present`] for a file that parses
//!   at this build's version and passes its validation, and [`Loaded::Damaged`]
//!   for anything else: a file that cannot be read, a link to nothing, text
//!   that is not strict JSON, a version this build does not read, or a record
//!   that fails its validation. A damaged file is never read as absent.
//! - **Nothing is written over damage.** [`Store::save`] validates the value
//!   first and then refuses to write while the file on disk is damaged, which
//!   leaves the evidence for the Architect. The one exception is a record whose
//!   policy is [`OnDamaged::Rebuild`].
//! - **Every call reads the disk.** Nothing is cached, so a repair, or a new
//!   damage, is seen at the next read.
//! - Writes go through `eph_fs::write_atomic` only (rule 5).
//!
//! Each record names its policy for a damaged file:
//! - [`OnDamaged::Refuse`]: the record reads as damaged, and every write is
//!   refused until the Architect repairs it.
//! - [`OnDamaged::Block`]: the same, and the caller also refuses the action that
//!   depends on the record. The store behaves as for Refuse; the policy travels
//!   with the [`Damage`] so the caller can tell.
//! - [`OnDamaged::Rebuild`]: the record is derived, and the caller rebuilds it.
//!   A file whose text is not a valid record is deleted when it is read. One
//!   that cannot be read at all is left in place, because what it holds is
//!   unknown, and a rebuild writes over it. Either way the damage is returned,
//!   saying which, so the caller reports it (R1.4a review, 2026-10-20).

use std::fmt;
use std::fs;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use eph_fs::atomic::write_atomic;
use eph_schema::degradation::{Cause, Source};
use eph_schema::document::{Versioned, parse, render};
use eph_schema::error::SchemaError;

/// What a record does when its file is damaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDamaged {
    /// Read as damaged; every write refused until a repair.
    Refuse,
    /// As Refuse, and the dependent action is refused too.
    Block,
    /// Derived: deleted, reported, and rebuilt.
    Rebuild,
}

/// A damaged record: which file, why, and what its policy does about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Damage {
    /// The file.
    pub file: PathBuf,
    /// Why it is damaged, in the reader's words.
    pub reason: String,
    /// The record's policy.
    pub policy: OnDamaged,
    /// Whether the read deleted the file: only under Rebuild, and only when
    /// its text was read and is not a valid record.
    pub deleted: bool,
}

impl Damage {
    /// Its cause in the degradation channel's grammar: `agora/damaged:<file>`.
    pub fn cause(&self) -> Cause {
        Cause {
            source: Source::Agora,
            slug: "damaged".to_string(),
            subject: self
                .file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        }
    }

    /// What the Architect reads: the file, why, and what happens until a repair.
    pub fn detail(&self) -> String {
        let consequence = match self.policy {
            OnDamaged::Refuse => {
                "it is left as evidence, and nothing is written to it until it is repaired"
            }
            OnDamaged::Block => {
                "it is left as evidence, nothing is written to it, and what depends on it is refused until it is repaired"
            }
            OnDamaged::Rebuild if self.deleted => "it is derived, so it was deleted and is rebuilt",
            OnDamaged::Rebuild => "it is derived, so it is rebuilt over, and it was not deleted",
        };
        format!(
            "{} is damaged ({}); {consequence}",
            self.file.display(),
            self.reason
        )
    }
}

/// What reading a record found.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded<T> {
    /// No file: the record's ordinary empty state.
    Absent,
    /// The record.
    Present(T),
    /// A file that is not this build's valid record.
    Damaged(Damage),
}

/// Why a write did not happen.
#[derive(Debug)]
pub enum StoreError {
    /// The file on disk is damaged, and the record's policy keeps it.
    Damaged(Damage),
    /// The value fails its own validation, so it was never written.
    Invalid(SchemaError),
    /// The write failed.
    Io(io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Damaged(damage) => write!(f, "refused: {}", damage.detail()),
            StoreError::Invalid(error) => {
                write!(f, "refused, as it would write an invalid record: {error}")
            }
            StoreError::Io(error) => write!(f, "the write failed: {error}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// One record's file and its policy.
#[derive(Debug, Clone)]
pub struct Store<T> {
    path: PathBuf,
    policy: OnDamaged,
    record: PhantomData<fn() -> T>,
}

impl<T: Versioned> Store<T> {
    /// The record `T` in `dir`, at its own file name, under `policy`.
    pub fn at(dir: &Path, policy: OnDamaged) -> Store<T> {
        Store {
            path: dir.join(T::FILE),
            policy,
            record: PhantomData,
        }
    }

    /// The record's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the record. See the module contract.
    pub fn load(&self) -> Loaded<T> {
        // The link itself, not its target: a link to nothing exists, and
        // reading it as absent would seed over it (R1.4a review, 2026-10-20).
        let read = match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::Absent,
            Err(error) => Err(error),
            Ok(_) => fs::read_to_string(&self.path),
        };
        match read {
            Err(error) => self.damaged(format!("it cannot be read: {error}"), false),
            Ok(text) => match parse::<T>(&text) {
                Ok(record) => Loaded::Present(record),
                Err(error) => self.damaged(error.to_string(), true),
            },
        }
    }

    /// Validates `record` and writes it, unless the file on disk is damaged and
    /// the policy keeps it. See the module contract.
    pub fn save(&self, record: &T) -> Result<(), StoreError> {
        let text = render(record).map_err(StoreError::Invalid)?;
        self.writable()?;
        write_atomic(&self.path, text.as_bytes()).map_err(StoreError::Io)
    }

    /// `Ok` when a write may happen: the file is absent, valid, or damaged
    /// under Rebuild, which a rebuild may replace.
    pub fn writable(&self) -> Result<(), StoreError> {
        match self.load() {
            Loaded::Damaged(damage) if damage.policy != OnDamaged::Rebuild => {
                Err(StoreError::Damaged(damage))
            }
            _ => Ok(()),
        }
    }

    /// The damage, after deleting the file when the policy is Rebuild and
    /// `deletable` (its text was read, so what is lost is known to be invalid).
    fn damaged(&self, reason: String, deletable: bool) -> Loaded<T> {
        let mut damage = Damage {
            file: self.path.clone(),
            reason,
            policy: self.policy,
            deleted: false,
        };
        if deletable && self.policy == OnDamaged::Rebuild {
            match fs::remove_file(&self.path) {
                Ok(()) => damage.deleted = true,
                Err(error) => {
                    damage.reason =
                        format!("{}; and it could not be deleted: {error}", damage.reason)
                }
            }
        }
        Loaded::Damaged(damage)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    /// A record for the store's own tests.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Note {
        schema_version: u32,
        text: String,
    }

    impl Versioned for Note {
        const FILE: &'static str = "note.json";
        const VERSION: u32 = 1;

        fn validate(&self) -> Result<(), SchemaError> {
            eph_schema::document::expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
            if self.text.is_empty() {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    "text",
                    "must not be empty",
                ));
            }
            Ok(())
        }
    }

    fn note(text: &str) -> Note {
        Note {
            schema_version: 1,
            text: text.to_string(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("eph-agora-store-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn plant(store: &Store<Note>, bytes: &str) {
        write_atomic(store.path(), bytes.as_bytes()).unwrap();
    }

    #[test]
    fn absent_present_and_damaged_are_three_answers() {
        let dir = scratch("three");
        let store = Store::<Note>::at(&dir, OnDamaged::Refuse);
        assert_eq!(store.path(), dir.join("note.json"));
        assert_eq!(store.load(), Loaded::Absent);
        store.save(&note("hello")).unwrap();
        assert_eq!(store.load(), Loaded::Present(note("hello")));
        for (bytes, says) in [
            ("{ not json", "not"),
            ("{\"schemaVersion\":9,\"text\":\"x\"}", "9"),
            ("{\"schemaVersion\":1,\"text\":\"\"}", "must not be empty"),
            (
                "{\"schemaVersion\":1,\"text\":\"a\",\"text\":\"b\"}",
                "twice",
            ),
        ] {
            plant(&store, bytes);
            let Loaded::Damaged(damage) = store.load() else {
                panic!("{bytes} read as something other than damaged");
            };
            assert!(damage.reason.contains(says), "{bytes}: {}", damage.reason);
            assert_eq!(damage.policy, OnDamaged::Refuse);
            assert_eq!(damage.file, store.path());
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_cannot_be_read_is_damaged_not_absent() {
        let dir = scratch("unreadable");
        let store = Store::<Note>::at(&dir, OnDamaged::Refuse);
        fs::create_dir_all(store.path()).unwrap();
        let Loaded::Damaged(damage) = store.load() else {
            panic!("a directory where the record goes read as absent or present");
        };
        assert!(
            damage.reason.starts_with("it cannot be read: "),
            "{}",
            damage.reason
        );
        write_atomic(&dir.join("bytes.json"), &[0xff, 0xfe, 0x00]).unwrap();
        fs::remove_dir_all(store.path()).unwrap();
        fs::rename(dir.join("bytes.json"), store.path()).unwrap();
        assert!(
            matches!(store.load(), Loaded::Damaged(_)),
            "bytes that are not UTF-8"
        );
        // A path the OS refuses to look up is an error other than "not found".
        let refused = Store::<Note>::at(&dir.join("a b"), OnDamaged::Refuse);
        assert!(matches!(refused.load(), Loaded::Damaged(_)));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_saved_with_a_byte_order_mark_is_damaged_and_says_how_to_fix_it() {
        let dir = scratch("bom");
        let store = Store::<Note>::at(&dir, OnDamaged::Refuse);
        plant(&store, "﻿{\"schemaVersion\":1,\"text\":\"x\"}");
        let Loaded::Damaged(damage) = store.load() else {
            panic!("a file with a byte-order mark read as something else");
        };
        assert!(
            damage.reason.contains(eph_schema::document::BOM),
            "{}",
            damage.reason
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_nothing_is_damaged_not_absent_and_is_never_deleted() {
        for policy in [OnDamaged::Refuse, OnDamaged::Rebuild] {
            let dir = scratch("dangling");
            let store = Store::<Note>::at(&dir, policy);
            std::os::unix::fs::symlink(dir.join("gone.json"), store.path()).unwrap();
            let Loaded::Damaged(damage) = store.load() else {
                panic!("{policy:?}: a link to nothing read as absent or present");
            };
            assert!(
                damage.reason.starts_with("it cannot be read: "),
                "{}",
                damage.reason
            );
            assert!(!damage.deleted);
            assert!(
                fs::symlink_metadata(store.path()).is_ok(),
                "{policy:?}: left in place"
            );
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn nothing_is_written_over_damage_under_refuse_or_block() {
        for policy in [OnDamaged::Refuse, OnDamaged::Block] {
            let dir = scratch("refuse");
            let store = Store::<Note>::at(&dir, policy);
            plant(&store, "{ evidence");
            let StoreError::Damaged(damage) = store.save(&note("over it")).unwrap_err() else {
                panic!("{policy:?}: a write over damage was not refused as damage");
            };
            assert_eq!(damage.policy, policy);
            assert_eq!(
                fs::read_to_string(store.path()).unwrap(),
                "{ evidence",
                "{policy:?}"
            );
            fs::remove_file(store.path()).unwrap();
            store.save(&note("after the repair")).unwrap();
            assert_eq!(store.load(), Loaded::Present(note("after the repair")));
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_derived_record_is_deleted_reported_and_rebuilt() {
        let dir = scratch("rebuild");
        let store = Store::<Note>::at(&dir, OnDamaged::Rebuild);
        plant(&store, "{ stale");
        let Loaded::Damaged(damage) = store.load() else {
            panic!("a damaged derived record read as something else");
        };
        assert_eq!(damage.policy, OnDamaged::Rebuild);
        assert!(damage.deleted);
        assert!(damage.detail().ends_with("it was deleted and is rebuilt"));
        assert!(!store.path().exists(), "deleted when read");
        assert_eq!(store.load(), Loaded::Absent);
        plant(&store, "{ stale again");
        store.save(&note("rebuilt")).unwrap();
        assert_eq!(store.load(), Loaded::Present(note("rebuilt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// R1.4a review (2026-10-20): a file that could not be read was deleted
    /// under Rebuild, though what it held was unknown.
    #[test]
    fn a_derived_record_that_cannot_be_read_is_left_in_place_and_written_over() {
        let dir = scratch("rebuild-unreadable");
        let store = Store::<Note>::at(&dir, OnDamaged::Rebuild);
        write_atomic(store.path(), &[0xff, 0xfe, 0x00]).unwrap();
        let Loaded::Damaged(damage) = store.load() else {
            panic!("bytes that are not UTF-8 read as a record");
        };
        assert!(
            damage.reason.starts_with("it cannot be read: "),
            "{}",
            damage.reason
        );
        assert!(!damage.deleted);
        assert!(
            damage
                .detail()
                .ends_with("it is rebuilt over, and it was not deleted"),
            "{}",
            damage.detail()
        );
        assert!(store.path().exists(), "left in place");
        store.save(&note("rebuilt")).unwrap();
        assert_eq!(store.load(), Loaded::Present(note("rebuilt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A directory without write permission refuses the delete on unix. A
    /// privileged user ignores the permission, so there the test says so and
    /// stops; the three CI platforms run unprivileged.
    #[cfg(unix)]
    #[test]
    fn a_derived_record_that_cannot_be_deleted_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("rebuild-stuck");
        let inner = dir.join("inner");
        fs::create_dir_all(&inner).unwrap();
        let store = Store::<Note>::at(&inner, OnDamaged::Rebuild);
        plant(&store, "{ stale");
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o555)).unwrap();
        let privileged = write_atomic(&inner.join("probe"), b"").is_ok();
        let loaded = store.load();
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o755)).unwrap();
        if privileged {
            eprintln!("skipped: this user ignores directory permissions");
            fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let Loaded::Damaged(damage) = loaded else {
            panic!("a damaged derived record read as something else");
        };
        assert!(
            damage.reason.contains("; and it could not be deleted: "),
            "{}",
            damage.reason
        );
        assert!(!damage.deleted);
        assert!(damage.detail().ends_with("and it was not deleted"));
        assert!(store.path().exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_invalid_value_is_never_written() {
        let dir = scratch("invalid");
        let store = Store::<Note>::at(&dir, OnDamaged::Refuse);
        store.save(&note("kept")).unwrap();
        let error = store.save(&note("")).unwrap_err();
        assert!(matches!(error, StoreError::Invalid(_)), "{error:?}");
        assert!(
            error
                .to_string()
                .starts_with("refused, as it would write an invalid record: ")
        );
        assert_eq!(store.load(), Loaded::Present(note("kept")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_that_fails_says_so() {
        let dir = scratch("io");
        let store = Store::<Note>::at(&dir.join("absent-dir"), OnDamaged::Refuse);
        let error = store.save(&note("x")).unwrap_err();
        assert!(matches!(error, StoreError::Io(_)), "{error:?}");
        assert!(
            error.to_string().starts_with("the write failed: "),
            "{error}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damage_names_its_cause_and_its_consequence() {
        let damage = |policy| Damage {
            file: PathBuf::from("agora").join("registry.json"),
            reason: "not JSON".to_string(),
            policy,
            deleted: policy == OnDamaged::Rebuild,
        };
        assert_eq!(
            damage(OnDamaged::Refuse).cause().to_string(),
            "agora/damaged:registry.json"
        );
        let refuse = damage(OnDamaged::Refuse).detail();
        assert!(
            refuse.contains("registry.json is damaged (not JSON)"),
            "{refuse}"
        );
        assert!(
            refuse.ends_with("nothing is written to it until it is repaired"),
            "{refuse}"
        );
        assert!(
            damage(OnDamaged::Block)
                .detail()
                .contains("what depends on it is refused")
        );
        assert!(
            damage(OnDamaged::Rebuild)
                .detail()
                .ends_with("it was deleted and is rebuilt")
        );
        let refused = StoreError::Damaged(damage(OnDamaged::Refuse)).to_string();
        assert!(refused.starts_with("refused: "), "{refused}");
    }
}

//! `Store<T>`: the one store for structured records (SDD-RUST §4.0 rule 6, P5).
//!
//! Contract:
//! - **Absent is not damaged.** [`Store::load`] answers [`Loaded::Absent`] for a
//!   file that does not exist, [`Loaded::Present`] for one that parses at this
//!   build's version and passes its validation, and [`Loaded::Damaged`] for
//!   anything else: a file that cannot be read, is not strict JSON, holds a
//!   version this build does not read, or fails its validation. A damaged file
//!   is never read as absent.
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
//! - [`OnDamaged::Rebuild`]: the record is derived. A damaged file is deleted
//!   when it is read, the damage is still returned so the caller reports it,
//!   and the caller rebuilds it.

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
            OnDamaged::Rebuild => "it is derived, so it was deleted and is rebuilt",
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
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::Absent,
            Err(error) => return self.damaged(format!("it cannot be read: {error}")),
        };
        match parse::<T>(&text) {
            Ok(record) => Loaded::Present(record),
            Err(error) => self.damaged(error.to_string()),
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
    /// under Rebuild (which [`Store::load`] has just deleted).
    pub fn writable(&self) -> Result<(), StoreError> {
        match self.load() {
            Loaded::Damaged(damage) if damage.policy != OnDamaged::Rebuild => {
                Err(StoreError::Damaged(damage))
            }
            _ => Ok(()),
        }
    }

    fn damaged(&self, reason: String) -> Loaded<T> {
        let reason = if self.policy == OnDamaged::Rebuild {
            match fs::remove_file(&self.path) {
                Ok(()) => reason,
                Err(error) => format!("{reason}; and it could not be deleted: {error}"),
            }
        } else {
            reason
        };
        Loaded::Damaged(Damage {
            file: self.path.clone(),
            reason,
            policy: self.policy,
        })
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
        fs::remove_dir_all(&dir).unwrap();
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
        assert!(!store.path().exists(), "deleted when read");
        assert_eq!(store.load(), Loaded::Absent);
        plant(&store, "{ stale again");
        store.save(&note("rebuilt")).unwrap();
        assert_eq!(store.load(), Loaded::Present(note("rebuilt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_derived_record_that_cannot_be_deleted_says_so() {
        let dir = scratch("rebuild-stuck");
        let store = Store::<Note>::at(&dir, OnDamaged::Rebuild);
        fs::create_dir_all(store.path().join("inside")).unwrap();
        let Loaded::Damaged(damage) = store.load() else {
            panic!("a directory read as a record");
        };
        assert!(
            damage.reason.contains("and it could not be deleted: "),
            "{}",
            damage.reason
        );
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

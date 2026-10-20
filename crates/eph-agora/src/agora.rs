//! `Agora`: the company's repository, opened in its boot order (ADR-0004;
//! SDD-RUST §2, §4.5).
//!
//! Contract: [`Agora::open`] runs the boot in this order, and every step that
//! fails short of the committer becomes a [`Condition`] in the degradation
//! grammar, never a silent default (invariant 7):
//! 1. **The committer** (`eph-git`): its lock, `git init` when there is no
//!    repository, and a killed harness's `index.lock`.
//! 2. **The reconcile**: whatever a killed harness left is committed under its
//!    own subject before anything else is written, so the history never calls
//!    crash leftovers a seed.
//! 3. **The files the harness owns, rendered at every boot**: `PROTOCOL.md`,
//!    from the bundled copy, so an improvement reaches every company that has
//!    already booted, and `.gitignore`, so the committer never sweeps a message
//!    in flight or a write's temporary file into history. A file whose bytes
//!    are already right is not rewritten.
//! 4. **The seeds, only when absent**: an empty roster, an empty ledger, and the
//!    bundled board. A damaged roster or ledger is reported and left as evidence.
//! 5. **One commit, "seed the Agora", only when the working tree changed.** A
//!    boot that changes nothing commits nothing.
//!
//! The roster and the ledger go through [`Store`] with the Refuse policy
//! (SDD-RUST §4.5). The ledger's writes apply its retention: done tasks past the
//! threshold are appended to `tasks-archive.jsonl` first, then the trimmed
//! ledger is written, so a write cut short between the two can leave a task in
//! both, never in neither. The board is free text with one scribe (FR-4.2): it
//! reads as empty when absent, which is not a degraded state.
//!
//! `PROTOCOL.md` is today's text: correcting what it teaches about the message
//! (`schemaVersion` in, `requires_reply` out) belongs to R5, which builds
//! Hermes and the message reader (DECISIONS-LOG 2026-10-20, *R1.4a —
//! implementation*).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use eph_fs::append::AppendLog;
use eph_fs::atomic::write_atomic;
use eph_fs::home::Condition;
use eph_git::committer::{Committer, Hooks, IndexLock, OpenError};
use eph_schema::degradation::{Cause, Source};
use eph_schema::document::Versioned;
use eph_schema::registry::Registry;
use eph_schema::tasks::{ArchivedTasks, Tasks, render_archived};
use eph_schema::time::Timestamp;

use crate::retention;
use crate::store::{Loaded, OnDamaged, Store, StoreError};

/// The agent-facing contract, bundled from the repository's `prompts/`.
pub const PROTOCOL: &str = include_str!("../../../prompts/agora/PROTOCOL.md");

/// The board's first text, bundled from the repository's `prompts/`.
pub const BOARD_SEED: &str = include_str!("../../../prompts/agora/board.md");

/// The Agora's `.gitignore`, rendered at every boot (SDD-RUST §2).
pub const GITIGNORE: &str = "# Written by the harness at every boot (SDD-RUST §2); an edit here is replaced.\n# A message in flight is a transient state the book of record already logs.\nagents/*/inbox/.inflight/\n# A write killed before its rename leaves its temporary file (eph-fs).\n.*.tmp\n";

/// The file names the harness owns in the Agora.
pub const PROTOCOL_FILE: &str = "PROTOCOL.md";
/// The board's file.
pub const BOARD_FILE: &str = "board.md";
/// The ignore file.
pub const GITIGNORE_FILE: &str = ".gitignore";

/// The subject the boot commits its own changes under.
pub const SEED_SUBJECT: &str = "seed the Agora";

/// An opened Agora, with what the boot found.
#[must_use]
#[derive(Debug)]
pub struct Booted {
    /// The Agora.
    pub agora: Agora,
    /// Whether this boot made the repository.
    pub initialized: bool,
    /// What the boot found wrong, for the degradation channel.
    pub conditions: Vec<Condition>,
}

/// Why the Agora could not be opened at all.
#[derive(Debug)]
pub enum BootError {
    /// The committer refused (another one holds the Agora, or `git init`
    /// failed), in its own words.
    Committer(OpenError),
    /// A file the boot must write could not be written.
    Write(StoreError),
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootError::Committer(error) => write!(f, "{error}"),
            BootError::Write(error) => write!(f, "the Agora could not be seeded: {error}"),
        }
    }
}

impl std::error::Error for BootError {}

/// What a ledger write did.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retained {
    /// How many done tasks it moved to the archive.
    pub archived: usize,
}

/// The Agora. See the module contract.
#[derive(Debug)]
pub struct Agora {
    root: PathBuf,
    committer: Committer,
}

impl Agora {
    /// Opens the Agora at `root` in its boot order. See the module contract.
    pub fn open(root: &Path, hooks: Hooks) -> Result<Booted, BootError> {
        let opened = Committer::open(root, hooks).map_err(BootError::Committer)?;
        let agora = Agora {
            root: opened.committer.root().to_path_buf(),
            committer: opened.committer,
        };
        let mut conditions = Vec::new();
        match opened.index_lock {
            IndexLock::Absent => {}
            IndexLock::Removed => conditions.push(condition(
                "index-lock",
                None,
                "a stale .git/index.lock, left by a harness that was killed, was removed at open"
                    .to_string(),
            )),
            IndexLock::Stuck(reason) => conditions.push(condition(
                "index-lock",
                None,
                format!(
                    "a stale .git/index.lock could not be removed ({reason}); commits retry and clear it once it is older than 20 s"
                ),
            )),
        }
        if let Err(failure) = agora.committer.reconcile().wait() {
            conditions.push(condition("commit", Some("reconcile"), failure.reason));
        }
        agora.render_owned()?;
        agora.seed(&mut conditions)?;
        if agora.committer.is_dirty()
            && let Err(failure) = agora.committer.commit(SEED_SUBJECT).wait()
        {
            conditions.push(condition("commit", Some("seed"), failure.reason));
        }
        Ok(Booted {
            agora,
            initialized: opened.initialized,
            conditions,
        })
    }

    /// The Agora's root, absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The single committer, for every commit the company makes.
    pub fn committer(&self) -> &Committer {
        &self.committer
    }

    /// The roster (SDD-RUST §4.5): absent, present, or damaged.
    pub fn registry(&self) -> Loaded<Registry> {
        self.registry_store().load()
    }

    /// Writes the roster, validated, never over a damaged file.
    pub fn write_registry(&self, registry: &Registry) -> Result<(), StoreError> {
        self.registry_store().save(registry)
    }

    /// The task ledger (SDD-RUST §4.5): absent, present, or damaged.
    pub fn tasks(&self) -> Loaded<Tasks> {
        self.tasks_store().load()
    }

    /// Writes the task ledger under its retention: validated first, refused over
    /// a damaged file, then the done tasks past the threshold appended to the
    /// archive, then the trimmed ledger written. See the module contract.
    pub fn write_tasks(&self, ledger: &Tasks) -> Result<Retained, StoreError> {
        let store = self.tasks_store();
        let (kept, archived) = retention::split(ledger);
        kept.validate().map_err(StoreError::Invalid)?;
        store.writable()?;
        if !archived.is_empty() {
            let mut archive = AppendLog::open(
                &self.root.join(ArchivedTasks::FILE),
                &self.root.join("tasks-archive"),
                u64::MAX,
                no_seq,
            )
            .map_err(StoreError::Io)?;
            let line = ArchivedTasks::new(archived.clone(), Timestamp::now());
            archive
                .append(|_seq| render_archived(&line))
                .map_err(|error| StoreError::Io(io::Error::other(format!("{error:?}"))))?;
        }
        store.save(&kept)?;
        Ok(Retained {
            archived: archived.len(),
        })
    }

    /// The board (FR-4.2). Absent reads as empty: a company that has not
    /// written to its board yet is not degraded.
    pub fn board(&self) -> io::Result<String> {
        match fs::read_to_string(self.root.join(BOARD_FILE)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            read => read,
        }
    }

    /// Writes the board whole, ending it with a newline.
    pub fn write_board(&self, body: &str) -> io::Result<()> {
        let mut text = body.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        write_atomic(&self.root.join(BOARD_FILE), text.as_bytes())
    }

    fn registry_store(&self) -> Store<Registry> {
        Store::at(&self.root, OnDamaged::Refuse)
    }

    fn tasks_store(&self) -> Store<Tasks> {
        Store::at(&self.root, OnDamaged::Refuse)
    }

    /// Renders the files the harness owns, writing only those whose bytes
    /// differ.
    fn render_owned(&self) -> Result<(), BootError> {
        for (name, text) in [(PROTOCOL_FILE, PROTOCOL), (GITIGNORE_FILE, GITIGNORE)] {
            let path = self.root.join(name);
            if fs::read(&path).is_ok_and(|bytes| bytes == text.as_bytes()) {
                continue;
            }
            write_atomic(&path, text.as_bytes())
                .map_err(|error| BootError::Write(StoreError::Io(error)))?;
        }
        Ok(())
    }

    /// Seeds what is absent, and reports what is damaged.
    fn seed(&self, conditions: &mut Vec<Condition>) -> Result<(), BootError> {
        seed_record(&self.registry_store(), &Registry::empty(), conditions)?;
        seed_record(&self.tasks_store(), &Tasks::empty(), conditions)?;
        let board = self.root.join(BOARD_FILE);
        if fs::symlink_metadata(&board).is_err() {
            write_atomic(&board, BOARD_SEED.as_bytes())
                .map_err(|error| BootError::Write(StoreError::Io(error)))?;
        }
        Ok(())
    }
}

fn seed_record<T: Versioned>(
    store: &Store<T>,
    empty: &T,
    conditions: &mut Vec<Condition>,
) -> Result<(), BootError> {
    match store.load() {
        Loaded::Absent => store.save(empty).map_err(BootError::Write),
        Loaded::Present(_) => Ok(()),
        Loaded::Damaged(damage) => {
            conditions.push(Condition {
                cause: damage.cause(),
                detail: damage.detail(),
            });
            Ok(())
        }
    }
}

/// The archive's lines carry no `seq`.
fn no_seq(_line: &str) -> Option<u64> {
    None
}

fn condition(slug: &str, subject: Option<&str>, detail: String) -> Condition {
    Condition {
        cause: Cause {
            source: Source::Agora,
            slug: slug.to_string(),
            subject: subject.map(str::to_string),
        },
        detail,
    }
}

#[cfg(test)]
mod tests {
    use eph_git::committer::FaultPoint;
    use eph_git::runner::Git;
    use eph_schema::document::parse;
    use eph_schema::tasks::TaskStatus;

    use super::*;
    use crate::retention::tests::{ledger, task};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-agora-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(root: &Path) -> Booted {
        Agora::open(root, Hooks::none()).unwrap()
    }

    /// git on the Agora, reading no machine configuration.
    fn git(root: &Path, args: &[&str]) -> String {
        let result = Git::target().without_machine_config().run(root, args);
        assert!(result.ok(), "git {args:?}: {result:?}");
        result.stdout
    }

    fn subjects(root: &Path) -> Vec<String> {
        git(root, &["log", "--format=%s"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn tracked(root: &Path, path: &str) -> bool {
        Git::target()
            .without_machine_config()
            .run(root, &["cat-file", "-e", &format!("HEAD:{path}")])
            .ok()
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(path, text.as_bytes()).unwrap();
    }

    fn causes(booted: &Booted) -> Vec<String> {
        booted
            .conditions
            .iter()
            .map(|condition| condition.cause.to_string())
            .collect()
    }

    #[test]
    fn a_first_boot_makes_seeds_and_commits_the_agora_once() {
        let dir = scratch("first");
        let root = dir.join("agora");
        let booted = open(&root);
        assert!(booted.initialized);
        assert!(booted.conditions.is_empty(), "{:?}", booted.conditions);
        assert_eq!(
            fs::read_to_string(root.join(PROTOCOL_FILE)).unwrap(),
            PROTOCOL
        );
        assert_eq!(
            fs::read_to_string(root.join(GITIGNORE_FILE)).unwrap(),
            GITIGNORE
        );
        assert_eq!(
            fs::read_to_string(root.join(BOARD_FILE)).unwrap(),
            BOARD_SEED
        );
        assert_eq!(booted.agora.registry(), Loaded::Present(Registry::empty()));
        assert_eq!(booted.agora.tasks(), Loaded::Present(Tasks::empty()));
        assert_eq!(subjects(&root), [SEED_SUBJECT]);
        for file in [
            PROTOCOL_FILE,
            GITIGNORE_FILE,
            BOARD_FILE,
            "registry.json",
            "tasks.json",
        ] {
            assert!(tracked(&root, file), "{file} is committed");
        }
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_that_changes_nothing_commits_nothing() {
        let dir = scratch("unchanged");
        let root = dir.join("agora");
        drop(open(&root));
        let protocol = fs::metadata(root.join(PROTOCOL_FILE))
            .unwrap()
            .modified()
            .unwrap();
        let again = open(&root);
        assert!(!again.initialized);
        assert!(again.conditions.is_empty(), "{:?}", again.conditions);
        assert_eq!(subjects(&root), [SEED_SUBJECT]);
        assert_eq!(
            fs::metadata(root.join(PROTOCOL_FILE))
                .unwrap()
                .modified()
                .unwrap(),
            protocol,
            "a file already right is not rewritten"
        );
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_a_killed_harness_left_is_reconciled_under_its_own_subject_first() {
        let dir = scratch("leftovers");
        let root = dir.join("agora");
        drop(open(&root));
        write(
            &root
                .join("agents")
                .join("mason")
                .join("outbox")
                .join("m1.json"),
            "{}\n",
        );
        drop(open(&root));
        assert_eq!(
            subjects(&root),
            [eph_git::committer::RECONCILE_SUBJECT, SEED_SUBJECT],
            "the leftovers are a reconcile, and the boot that found them seeded nothing"
        );
        assert!(tracked(&root, "agents/mason/outbox/m1.json"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_protocol_from_an_older_build_is_rendered_again_and_committed_as_a_seed() {
        let dir = scratch("protocol");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join(PROTOCOL_FILE), "an old protocol\n");
        git(
            &root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-am",
                "an older build",
            ],
        );
        drop(open(&root));
        assert_eq!(
            fs::read_to_string(root.join(PROTOCOL_FILE)).unwrap(),
            PROTOCOL
        );
        assert_eq!(subjects(&root)[0], SEED_SUBJECT);
        let seeded = git(&root, &["show", "--name-only", "--format=", "HEAD"]);
        assert_eq!(seeded.trim(), PROTOCOL_FILE);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_ignore_file_keeps_messages_in_flight_and_temporary_files_out_of_history() {
        let dir = scratch("ignore");
        let root = dir.join("agora");
        write(&root.join(GITIGNORE_FILE), "a hand edit\n");
        let booted = open(&root);
        assert_eq!(
            fs::read_to_string(root.join(GITIGNORE_FILE)).unwrap(),
            GITIGNORE
        );
        write(&root.join("agents/mason/inbox/.inflight/m1.json"), "{}\n");
        write(
            &root.join("agents/mason/inbox/.m2.json.0a1b2c3d.tmp"),
            "half",
        );
        write(&root.join("agents/mason/inbox/m3.json"), "{}\n");
        booted.agora.committer().commit("deliver").wait().unwrap();
        assert!(tracked(&root, "agents/mason/inbox/m3.json"));
        assert!(!tracked(&root, "agents/mason/inbox/.inflight/m1.json"));
        assert!(!tracked(&root, "agents/mason/inbox/.m2.json.0a1b2c3d.tmp"));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stale_index_lock_is_removed_at_open_and_reported() {
        let dir = scratch("index-lock");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join(".git").join("index.lock"), "");
        let booted = open(&root);
        assert_eq!(causes(&booted), ["agora/index-lock"]);
        assert!(booted.conditions[0].detail.contains("was removed at open"));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_held_index_lock_is_reported_as_stuck_on_windows() {
        let dir = scratch("index-lock-held");
        let root = dir.join("agora");
        drop(open(&root));
        let held = eph_fs::lock::Exclusive::try_acquire(&root.join(".git").join("index.lock"))
            .unwrap()
            .unwrap();
        let booted = open(&root);
        assert!(causes(&booted).contains(&"agora/index-lock".to_string()));
        assert!(
            booted.conditions[0].detail.contains("could not be removed"),
            "{:?}",
            booted.conditions
        );
        drop(held);
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_roster_or_ledger_is_reported_and_left_as_evidence() {
        let dir = scratch("damaged");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join("registry.json"), "{ evidence");
        write(
            &root.join("tasks.json"),
            "{\"schemaVersion\":1,\"tasks\":[]}",
        );
        let booted = open(&root);
        assert_eq!(
            causes(&booted),
            ["agora/damaged:registry.json", "agora/damaged:tasks.json"]
        );
        assert_eq!(
            fs::read_to_string(root.join("registry.json")).unwrap(),
            "{ evidence"
        );
        let refused = booted.agora.write_registry(&Registry::empty()).unwrap_err();
        assert!(matches!(refused, StoreError::Damaged(_)), "{refused:?}");
        let refused = booted.agora.write_tasks(&Tasks::empty()).unwrap_err();
        assert!(matches!(refused, StoreError::Damaged(_)), "{refused:?}");
        assert!(!root.join(ArchivedTasks::FILE).exists());
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_reconcile_or_seed_that_cannot_commit_is_reported_and_the_boot_goes_on() {
        let dir = scratch("cannot-commit");
        let root = dir.join("agora");
        let hooks = Hooks::none().with_faults(|point| {
            if point == FaultPoint::BeforeStage {
                return Err("git is unwell".to_string());
            }
            Ok(())
        });
        let booted = Agora::open(&root, hooks).unwrap();
        assert_eq!(
            causes(&booted),
            ["agora/commit:reconcile", "agora/commit:seed"]
        );
        assert!(
            booted
                .conditions
                .iter()
                .all(|c| c.detail == "git is unwell")
        );
        assert_eq!(
            booted.agora.registry(),
            Loaded::Present(Registry::empty()),
            "the seeds are on disk"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_held_agora_is_refused_in_the_committers_words() {
        let dir = scratch("held");
        let root = dir.join("agora");
        let first = open(&root);
        let refused = Agora::open(&root, Hooks::none()).unwrap_err();
        assert!(
            matches!(refused, BootError::Committer(OpenError::Held(_))),
            "{refused:?}"
        );
        assert!(
            refused
                .to_string()
                .contains("another committer holds this Agora"),
            "{refused}"
        );
        drop(first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_seed_that_cannot_be_written_stops_the_boot_and_says_why() {
        let dir = scratch("cannot-seed");
        let root = dir.join("agora");
        drop(open(&root));
        fs::remove_file(root.join(BOARD_FILE)).unwrap();
        fs::create_dir_all(root.join(BOARD_FILE).with_file_name("board.md.d")).unwrap();
        // A directory where the protocol goes: the render cannot replace it.
        fs::remove_file(root.join(PROTOCOL_FILE)).unwrap();
        fs::create_dir_all(root.join(PROTOCOL_FILE).join("inside")).unwrap();
        let refused = Agora::open(&root, Hooks::none()).unwrap_err();
        assert!(
            matches!(refused, BootError::Write(StoreError::Io(_))),
            "{refused:?}"
        );
        assert!(
            refused
                .to_string()
                .starts_with("the Agora could not be seeded: "),
            "{refused}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_roster_and_the_ledger_round_trip_through_their_stores() {
        let dir = scratch("round-trip");
        let root = dir.join("agora");
        let booted = open(&root);
        let agora = &booted.agora;
        agora.write_registry(&Registry::empty()).unwrap();
        let mut done = task(1, true, 2_000);
        done.status = TaskStatus::Done;
        let tasks = ledger(vec![done, task(2, false, 1_000)]);
        assert_eq!(agora.write_tasks(&tasks).unwrap(), Retained { archived: 0 });
        assert_eq!(agora.tasks(), Loaded::Present(tasks));
        let mut twice = ledger(vec![task(3, false, 1_000), task(3, false, 1_000)]);
        twice.tasks[1].title = "the same id".into();
        let refused = agora.write_tasks(&twice).unwrap_err();
        assert!(matches!(refused, StoreError::Invalid(_)), "{refused:?}");
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_ledger_past_its_threshold_archives_the_old_done_first_then_trims() {
        let dir = scratch("retention");
        let root = dir.join("agora");
        let booted = open(&root);
        let tasks = ledger(
            (0..9_001)
                .map(|n| task(n, true, 2_000 + i64::from(n)))
                .collect(),
        );
        let retained = booted.agora.write_tasks(&tasks).unwrap();
        assert_eq!(retained, Retained { archived: 8_001 });
        let Loaded::Present(kept) = booted.agora.tasks() else {
            panic!("the trimmed ledger did not read back");
        };
        assert_eq!(kept.tasks.len(), 1_000);
        let archive = fs::read_to_string(root.join(ArchivedTasks::FILE)).unwrap();
        let lines: Vec<&str> = archive.lines().collect();
        assert_eq!(lines.len(), 1, "one line per retention run");
        let line = parse::<ArchivedTasks>(lines[0]).unwrap();
        assert_eq!(line.tasks.len(), 8_001);
        assert_eq!(line.tasks[0].title, "task 0");
        assert!(
            line.tasks
                .iter()
                .all(|task| task.status == TaskStatus::Done)
        );
        // A second run appends a second line; the first is never rewritten.
        let mut grown = kept.clone();
        grown
            .tasks
            .extend((9_001..17_002).map(|n| task(n, true, 20_000 + i64::from(n))));
        assert_eq!(
            booted.agora.write_tasks(&grown).unwrap(),
            Retained { archived: 8_001 }
        );
        let archive = fs::read_to_string(root.join(ArchivedTasks::FILE)).unwrap();
        assert_eq!(archive.lines().count(), 2);
        assert!(archive.starts_with(lines[0]), "append-only");
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_board_reads_empty_when_absent_and_is_written_whole() {
        let dir = scratch("board");
        let root = dir.join("agora");
        let booted = open(&root);
        let agora = &booted.agora;
        assert_eq!(agora.board().unwrap(), BOARD_SEED);
        fs::remove_file(root.join(BOARD_FILE)).unwrap();
        assert_eq!(agora.board().unwrap(), "", "absent is not degraded");
        agora.write_board("## Now\n- ship R1.4a").unwrap();
        assert_eq!(agora.board().unwrap(), "## Now\n- ship R1.4a\n");
        agora.write_board("ends\n").unwrap();
        assert_eq!(agora.board().unwrap(), "ends\n");
        fs::create_dir_all(root.join(BOARD_FILE).with_extension("tmp")).unwrap();
        fs::remove_file(root.join(BOARD_FILE)).unwrap();
        fs::create_dir_all(root.join(BOARD_FILE)).unwrap();
        assert!(
            agora.board().is_err(),
            "a board that cannot be read is an error, not empty"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_error_says_which_step_refused() {
        let write = BootError::Write(StoreError::Io(io::Error::other("disk full")));
        assert_eq!(
            write.to_string(),
            "the Agora could not be seeded: the write failed: disk full"
        );
    }
}

//! `Agora`: the company's repository, opened in its boot order (ADR-0004;
//! SDD-RUST §2, §4.5, §10).
//!
//! Contract: [`Agora::open`] fails only when the committer refuses: another one
//! holds the Agora, or `git init` failed. Every later step that goes wrong
//! becomes a [`Condition`] in the degradation grammar, and the boot goes on
//! (invariant 7). The order:
//! 1. **The committer** (`eph-git`): its lock, `git init` when there is no
//!    repository (after the lock files a killed `git init` left, which are
//!    reported), and an `index.lock` a killed git left. A repository made over
//!    a directory that already held files is reported, since whatever history
//!    it had is gone.
//! 2. **`.gitignore`, rendered** before anything is committed, so the reconcile
//!    never sweeps a message in flight or a killed write's temporary file into
//!    history.
//! 3. **The reconcile**: whatever a killed harness left is committed under its
//!    own subject, so the history never calls crash leftovers a seed. A
//!    repository this boot made over an empty directory has nothing to
//!    reconcile.
//! 4. **`PROTOCOL.md`, rendered** from the home's override or the bundled copy
//!    (SDD-RUST §4.4, §4.6), so an improvement reaches every company that has
//!    already booted.
//! 5. **The seeds, only when absent**: an empty roster, an empty ledger, and
//!    the board (the home's override or the bundled text). A damaged roster or
//!    ledger is reported and left as evidence.
//! 6. **One commit, when this boot wrote anything**, under [`SEED_SUBJECT`], or
//!    under [`RECONCILE_AND_SEED_SUBJECT`] when the reconcile failed, since the
//!    leftovers are then in it. A boot that writes nothing commits nothing.
//!
//! A file the harness owns is rewritten only when it is not a regular file
//! holding exactly the right bytes. The roster and the ledger go through
//! [`Store`] with the Refuse policy (SDD-RUST §4.5). A ledger write validates
//! the whole ledger, then applies the retention: the done tasks past the
//! threshold are appended to `tasks-archive.jsonl` as one line, then the
//! trimmed ledger is written, so a write cut short between the two can leave a
//! task in both files, never in neither. The board is free text with one scribe
//! (FR-4.2): it reads as empty when absent, which is not a degraded state.
//!
//! `PROTOCOL.md` is today's text: correcting what it teaches about the message
//! (`schemaVersion` in, `requires_reply` out) belongs to R5, which builds
//! Hermes and the message reader (Architect, DECISIONS-LOG 2026-10-25).

use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use eph_fs::append::append_line;
use eph_fs::atomic::write_atomic;
use eph_fs::home::{Condition, prompt};
use eph_git::committer::{Committer, Hooks, IndexLock, OpenError, STALE_AFTER};
use eph_schema::degradation::{Cause, Source};
use eph_schema::document::Versioned;
use eph_schema::registry::Registry;
use eph_schema::tasks::{ArchivedTasks, Task, Tasks, render_archived};
use eph_schema::time::Timestamp;

use crate::retention;
use crate::store::{Loaded, OnDamaged, Store, StoreError};

/// The agent-facing contract, bundled from the repository's `prompts/`.
pub const PROTOCOL: &str = include_str!("../../../prompts/agora/PROTOCOL.md");

/// The board's first text, bundled from the repository's `prompts/`.
pub const BOARD_SEED: &str = include_str!("../../../prompts/agora/board.md");

/// Where the home's override of [`PROTOCOL`] goes, under its `prompts/`.
pub const PROTOCOL_PROMPT: &str = "agora/PROTOCOL.md";

/// Where the home's override of [`BOARD_SEED`] goes, under its `prompts/`.
pub const BOARD_PROMPT: &str = "agora/board.md";

/// The Agora's `.gitignore`, rendered at every boot (SDD-RUST §2, §4.5). Its
/// second pattern is `write_atomic`'s temporary name, `.<name>.<8 hex>.tmp`,
/// and nothing wider, so an agent's own `.notes.tmp` is still committed.
pub const GITIGNORE: &str = concat!(
    "# Written by the harness at every boot (SDD-RUST §2); an edit here is replaced.\n",
    "# A message in flight is a transient state the book of record already logs.\n",
    "agents/*/inbox/.inflight/\n",
    "# A write killed before its rename leaves .<name>.<8 hex>.tmp beside its file (eph-fs).\n",
    ".*.[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f].tmp\n",
);

/// The protocol's file.
pub const PROTOCOL_FILE: &str = "PROTOCOL.md";
/// The board's file.
pub const BOARD_FILE: &str = "board.md";
/// The ignore file.
pub const GITIGNORE_FILE: &str = ".gitignore";

/// The subject the boot commits what it wrote under.
pub const SEED_SUBJECT: &str = "seed the Agora";

/// The subject when the reconcile failed and the boot then wrote something:
/// the one commit holds the leftovers too, so it names them.
pub const RECONCILE_AND_SEED_SUBJECT: &str =
    "reconcile uncommitted work after restart, and seed the Agora";

/// What a failed render leaves.
const RENDER_LATER: &str = "what is on disk, if anything, stays until a later boot renders it";

/// What a failed seed leaves.
const SEED_LATER: &str = "it stays absent until a later boot seeds it";

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

/// What a ledger write did.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retained {
    /// How many done tasks it moved to the archive.
    pub archived: usize,
    /// The ledger as it was written.
    pub kept: Tasks,
}

/// The Agora. See the module contract.
#[derive(Debug)]
pub struct Agora {
    root: PathBuf,
    committer: Committer,
}

impl Agora {
    /// Opens the Agora at `root` in its boot order, with the home's prompt
    /// overrides in `prompts`. See the module contract.
    pub fn open(root: &Path, prompts: &Path, hooks: Hooks) -> Result<Booted, OpenError> {
        let held = holds_files(root);
        let opened = Committer::open(root, hooks)?;
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
                "a .git/index.lock that was still there after the open's wait was removed; a git killed mid-commit leaves one".to_string(),
            )),
            IndexLock::Stuck(reason) => conditions.push(condition(
                "index-lock",
                None,
                format!(
                    "a .git/index.lock could not be removed at open ({reason}); a later commit removes it once it is older than {} s, if nothing still holds it",
                    STALE_AFTER.as_secs()
                ),
            )),
        }
        if !opened.init_locks.is_empty() {
            conditions.push(condition(
                "init-lock",
                None,
                format!(
                    "a git init killed before it finished had left {} in .git; removed, and the repository was made",
                    opened.init_locks.join(", ")
                ),
            ));
        }
        if opened.initialized && held {
            conditions.push(condition(
                "history-lost",
                None,
                format!(
                    "{} held files but no repository, so a new one was made: whatever history it had is gone, and the files are committed as found",
                    agora.root.display()
                ),
            ));
        }
        let ignore = agora.render(GITIGNORE_FILE, GITIGNORE, &mut conditions);
        let reconcile_failed = if opened.initialized && !held {
            false
        } else if let Err(failure) = agora.committer.reconcile().wait() {
            conditions.push(condition("commit", Some("reconcile"), failure.reason));
            true
        } else {
            false
        };
        let protocol = prompt(prompts, PROTOCOL_PROMPT, PROTOCOL);
        conditions.extend(protocol.condition);
        let rendered = agora.render(PROTOCOL_FILE, &protocol.text, &mut conditions);
        let seeded = agora.seed(prompts, &mut conditions);
        // A `.gitignore` the reconcile already committed costs one commit that
        // lands with nothing staged.
        if ignore || rendered || seeded {
            let subject = if reconcile_failed {
                RECONCILE_AND_SEED_SUBJECT
            } else {
                SEED_SUBJECT
            };
            if let Err(failure) = agora.committer.commit(subject).wait() {
                conditions.push(condition("commit", Some("seed"), failure.reason));
            }
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

    /// Writes the task ledger under its retention: the whole ledger validated,
    /// refused over a damaged file, then the done tasks past the threshold
    /// appended to the archive, then the trimmed ledger written. See the module
    /// contract.
    pub fn write_tasks(&self, ledger: &Tasks) -> Result<Retained, StoreError> {
        ledger.validate().map_err(StoreError::Invalid)?;
        let store = self.tasks_store();
        store.writable()?;
        let (kept, archived) = retention::split(ledger);
        // The split keeps every dependency of a kept task, so this holds by
        // construction. Checking it before the first write means a fault in
        // the split refuses the write instead of archiving tasks the ledger
        // then keeps too.
        kept.validate().map_err(StoreError::Invalid)?;
        let moved = archived.len();
        if !archived.is_empty() {
            self.archive(archived)?;
        }
        store.save(&kept)?;
        Ok(Retained {
            archived: moved,
            kept,
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

    /// Appends one archive line holding `tasks`. The archive's lines carry no
    /// `seq` and it never rotates, so only its last byte is read (Architect,
    /// 2026-10-25).
    fn archive(&self, tasks: Vec<Task>) -> Result<(), StoreError> {
        let line = render_archived(&ArchivedTasks::new(tasks, Timestamp::now()))
            .map_err(StoreError::Invalid)?;
        append_line(&self.root.join(ArchivedTasks::FILE), &line).map_err(StoreError::Io)
    }

    /// Writes `text` to the owned file `name` unless it is a regular file that
    /// already holds exactly those bytes, and says whether it wrote. A link is
    /// replaced too: git does not follow a `.gitignore` that is a link. A
    /// failure becomes a condition.
    fn render(&self, name: &str, text: &str, conditions: &mut Vec<Condition>) -> bool {
        let path = self.root.join(name);
        let right = fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file())
            && fs::read(&path).is_ok_and(|bytes| bytes == text.as_bytes());
        if right {
            return false;
        }
        match write_atomic(&path, text.as_bytes()) {
            Ok(()) => true,
            Err(error) => {
                conditions.push(failed("render", name, &error, RENDER_LATER));
                false
            }
        }
    }

    /// Seeds what is absent, reports what is damaged, and says whether it
    /// wrote anything.
    fn seed(&self, prompts: &Path, conditions: &mut Vec<Condition>) -> bool {
        let roster = seed_record(&self.registry_store(), &Registry::empty(), conditions);
        let ledger = seed_record(&self.tasks_store(), &Tasks::empty(), conditions);
        let path = self.root.join(BOARD_FILE);
        let mut board = false;
        if fs::symlink_metadata(&path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound) {
            let seed = prompt(prompts, BOARD_PROMPT, BOARD_SEED);
            conditions.extend(seed.condition);
            match write_atomic(&path, seed.text.as_bytes()) {
                Ok(()) => board = true,
                Err(error) => conditions.push(failed("seed", BOARD_FILE, &error, SEED_LATER)),
            }
        }
        roster || ledger || board
    }
}

/// Writes `empty` when the record is absent, reports it when it is damaged,
/// and says whether it wrote.
fn seed_record<T: Versioned>(store: &Store<T>, empty: &T, conditions: &mut Vec<Condition>) -> bool {
    match store.load() {
        Loaded::Absent => match store.save(empty) {
            Ok(()) => true,
            Err(error) => {
                conditions.push(failed("seed", T::FILE, &error, SEED_LATER));
                false
            }
        },
        Loaded::Present(_) => false,
        Loaded::Damaged(damage) => {
            conditions.push(Condition {
                cause: damage.cause(),
                detail: damage.detail(),
            });
            false
        }
    }
}

/// Whether `root` holds anything besides `.git`. A directory that cannot be
/// listed counts as holding files, so its boot reconciles instead of assuming
/// it is new.
fn holds_files(root: &Path) -> bool {
    match fs::read_dir(root) {
        Err(error) => error.kind() != io::ErrorKind::NotFound,
        Ok(entries) => entries.flatten().any(|entry| entry.file_name() != ".git"),
    }
}

fn failed(slug: &str, name: &str, error: &dyn Display, consequence: &str) -> Condition {
    condition(
        slug,
        Some(name),
        format!("{name} could not be written ({error}); {consequence}"),
    )
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
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use eph_git::committer::{FaultPoint, RECONCILE_SUBJECT};
    use eph_git::runner::Git;
    use eph_schema::document::parse;
    use eph_schema::registry::{Agent, EngineId, HireRef};
    use eph_schema::tasks::TaskStatus;

    use super::*;
    use crate::retention::tests::{ledger, task};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-agora-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The home's `prompts/`, beside the Agora, as in a real home.
    fn prompts(root: &Path) -> PathBuf {
        root.with_file_name("prompts")
    }

    fn open(root: &Path) -> Booted {
        Agora::open(root, &prompts(root), Hooks::none()).unwrap()
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

    fn commit_by_hand(root: &Path, subject: &str) {
        git(
            root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-qam",
                subject,
            ],
        );
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

    /// Faults every `BeforeStage`.
    fn failing() -> Hooks {
        Hooks::none().with_faults(|point| {
            if point == FaultPoint::BeforeStage {
                return Err("git is unwell".to_string());
            }
            Ok(())
        })
    }

    /// Faults only the first `BeforeStage`.
    fn failing_once() -> Hooks {
        let seen = AtomicUsize::new(0);
        Hooks::none().with_faults(move |point| {
            if point == FaultPoint::BeforeStage && seen.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err("git is unwell".to_string());
            }
            Ok(())
        })
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
        assert_eq!(
            subjects(&root),
            [SEED_SUBJECT],
            "a repository made over an empty directory has nothing to reconcile"
        );
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
        write(&root.join("agents/mason/outbox/m1.json"), "{}\n");
        drop(open(&root));
        assert_eq!(
            subjects(&root),
            [RECONCILE_SUBJECT, SEED_SUBJECT],
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
        commit_by_hand(&root, "an older build");
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
    fn the_ignore_file_keeps_messages_in_flight_and_atomic_temporaries_out_of_history() {
        let dir = scratch("ignore");
        let root = dir.join("agora");
        write(&root.join(GITIGNORE_FILE), "a hand edit\n");
        let booted = open(&root);
        assert_eq!(
            fs::read_to_string(root.join(GITIGNORE_FILE)).unwrap(),
            GITIGNORE
        );
        let inbox = root.join("agents/mason/inbox");
        write(&inbox.join(".inflight/m1.json"), "{}\n");
        write(&inbox.join(".m2.json.0a1b2c3d.tmp"), "half");
        write(&inbox.join("m3.json"), "{}\n");
        write(&inbox.join(".notes.tmp"), "an agent's own file\n");
        booted.agora.committer().commit("deliver").wait().unwrap();
        assert!(tracked(&root, "agents/mason/inbox/m3.json"));
        assert!(!tracked(&root, "agents/mason/inbox/.inflight/m1.json"));
        assert!(!tracked(&root, "agents/mason/inbox/.m2.json.0a1b2c3d.tmp"));
        assert!(
            tracked(&root, "agents/mason/inbox/.notes.tmp"),
            "only write_atomic's own temporary name is ignored"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// R1.4a review (2026-10-20): the reconcile ran before the render, so an
    /// older `.gitignore` let it commit a killed write's temporary file.
    #[test]
    fn the_ignore_file_is_rendered_before_the_reconcile_so_crash_residue_stays_out() {
        let dir = scratch("ignore-first");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join(GITIGNORE_FILE), "agents/*/inbox/.inflight/\n");
        commit_by_hand(&root, "an older build");
        let outbox = root.join("agents/mason/outbox");
        write(&outbox.join("m1.json"), "{}\n");
        write(&outbox.join(".m1.json.0a1b2c3d.tmp"), "half");
        drop(open(&root));
        assert!(tracked(&root, "agents/mason/outbox/m1.json"));
        assert!(!tracked(&root, "agents/mason/outbox/.m1.json.0a1b2c3d.tmp"));
        assert_eq!(
            git(&root, &["show", &format!("HEAD:{GITIGNORE_FILE}")]),
            GITIGNORE
        );
        assert_eq!(
            subjects(&root),
            [RECONCILE_SUBJECT, "an older build", SEED_SUBJECT],
            "the render rides in the reconcile, and nothing else is committed"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_owned_file_that_is_a_link_is_replaced_by_a_regular_one() {
        let dir = scratch("ignore-link");
        let root = dir.join("agora");
        drop(open(&root));
        write(&dir.join("elsewhere"), GITIGNORE);
        fs::remove_file(root.join(GITIGNORE_FILE)).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), root.join(GITIGNORE_FILE)).unwrap();
        drop(open(&root));
        let meta = fs::symlink_metadata(root.join(GITIGNORE_FILE)).unwrap();
        assert!(meta.is_file(), "git does not follow a linked .gitignore");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_git_init_a_kill_left_unfinished_is_reported_and_the_boot_goes_on() {
        let dir = scratch("init-lock");
        let root = dir.join("agora");
        write(&root.join(".git").join("config.lock"), "[core]\n");
        let booted = open(&root);
        assert!(booted.initialized);
        assert_eq!(causes(&booted), ["agora/init-lock"]);
        assert!(
            booted.conditions[0]
                .detail
                .contains("had left config.lock in .git; removed"),
            "{}",
            booted.conditions[0].detail
        );
        assert_eq!(subjects(&root), [SEED_SUBJECT], "the boot went on");
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
        assert!(
            booted.conditions[0]
                .detail
                .contains("was still there after the open's wait was removed"),
            "{}",
            booted.conditions[0].detail
        );
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
        let detail = &booted.conditions[0].detail;
        assert!(detail.contains("could not be removed"), "{detail}");
        assert!(
            detail.ends_with("once it is older than 20 s, if nothing still holds it"),
            "{detail}"
        );
        drop(held);
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_repository_made_over_existing_files_says_its_history_is_gone() {
        let dir = scratch("history-lost");
        let root = dir.join("agora");
        write(&root.join("notes.md"), "kept\n");
        let booted = open(&root);
        assert!(booted.initialized);
        assert_eq!(causes(&booted), ["agora/history-lost"]);
        assert!(
            booted.conditions[0]
                .detail
                .contains("whatever history it had is gone")
        );
        assert!(tracked(&root, "notes.md"));
        assert_eq!(subjects(&root), [SEED_SUBJECT, RECONCILE_SUBJECT]);
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_agora_emptied_by_hand_is_reconciled_not_taken_for_new() {
        let dir = scratch("emptied");
        let root = dir.join("agora");
        drop(open(&root));
        for entry in fs::read_dir(&root).unwrap().flatten() {
            if entry.file_name() != ".git" {
                let path = entry.path();
                if path.is_dir() {
                    fs::remove_dir_all(path).unwrap();
                } else {
                    fs::remove_file(path).unwrap();
                }
            }
        }
        let booted = open(&root);
        assert!(!booted.initialized);
        assert!(booted.conditions.is_empty(), "{:?}", booted.conditions);
        assert_eq!(
            subjects(&root),
            [SEED_SUBJECT, RECONCILE_SUBJECT, SEED_SUBJECT],
            "the deletions are recorded, then the seeds again"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn each_seed_is_written_only_when_absent_and_committed_as_a_seed() {
        for file in ["registry.json", "tasks.json", BOARD_FILE] {
            let dir = scratch("each-seed");
            let root = dir.join("agora");
            drop(open(&root));
            fs::remove_file(root.join(file)).unwrap();
            let booted = open(&root);
            assert!(
                booted.conditions.is_empty(),
                "{file}: {:?}",
                booted.conditions
            );
            assert!(root.join(file).is_file(), "{file} seeded again");
            assert!(tracked(&root, file), "{file}");
            assert_eq!(
                subjects(&root),
                [SEED_SUBJECT, RECONCILE_SUBJECT, SEED_SUBJECT],
                "{file}: its deletion is a reconcile, its seed a seed"
            );
            drop(booted);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// R1.4a review (2026-10-20): "seeded only when absent" had no test that
    /// a reboot keeps what the company wrote.
    #[test]
    fn a_reboot_keeps_the_companys_own_roster_ledger_and_board() {
        let dir = scratch("keeps");
        let root = dir.join("agora");
        let booted = open(&root);
        let roster = Registry {
            schema_version: Registry::VERSION,
            orchestrator_id: None,
            agents: BTreeMap::from([(
                "agent.mason".parse().unwrap(),
                Agent {
                    name: "Mason".into(),
                    role: "builder".into(),
                    engine: EngineId::Claude,
                    capabilities: vec!["rust".into()],
                    seat: "desk-1".into(),
                    env_grants: Vec::new(),
                    profile: None,
                    cwd: "C:/work/x".into(),
                    target: None,
                    budget: None,
                    hire: HireRef {
                        template: "builder".parse().unwrap(),
                        version: 1,
                    },
                    spawned_at: Timestamp::from_millis(1_000).unwrap(),
                    retired_at: None,
                },
            )]),
        };
        booted.agora.write_registry(&roster).unwrap();
        let tasks = ledger(vec![task(1, false, 2_000)]);
        let _written = booted.agora.write_tasks(&tasks).unwrap();
        booted
            .agora
            .write_board("## Now\n- hire a reviewer\n")
            .unwrap();
        booted
            .agora
            .committer()
            .commit("the company's work")
            .wait()
            .unwrap();
        drop(booted);
        let again = open(&root);
        assert!(again.conditions.is_empty(), "{:?}", again.conditions);
        assert_eq!(again.agora.registry(), Loaded::Present(roster));
        assert_eq!(again.agora.tasks(), Loaded::Present(tasks));
        assert_eq!(again.agora.board().unwrap(), "## Now\n- hire a reviewer\n");
        assert_eq!(subjects(&root), ["the company's work", SEED_SUBJECT]);
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_override_in_the_homes_prompts_replaces_the_bundled_protocol_and_board() {
        let dir = scratch("overrides");
        let root = dir.join("agora");
        let overrides = prompts(&root);
        write(&overrides.join("agora/PROTOCOL.md"), "our protocol\n");
        write(&overrides.join("agora/board.md"), "our board\n");
        drop(open(&root));
        assert_eq!(
            fs::read_to_string(root.join(PROTOCOL_FILE)).unwrap(),
            "our protocol\n"
        );
        assert_eq!(
            fs::read_to_string(root.join(BOARD_FILE)).unwrap(),
            "our board\n"
        );
        // Without the override the bundled protocol is rendered again; the
        // board was a seed, so it stays the company's.
        fs::remove_dir_all(&overrides).unwrap();
        drop(open(&root));
        assert_eq!(
            fs::read_to_string(root.join(PROTOCOL_FILE)).unwrap(),
            PROTOCOL
        );
        assert_eq!(
            fs::read_to_string(root.join(BOARD_FILE)).unwrap(),
            "our board\n"
        );
        // An override that cannot be read is reported, and the bundled is used.
        fs::create_dir_all(overrides.join("agora/PROTOCOL.md/inside")).unwrap();
        let booted = open(&root);
        assert_eq!(
            causes(&booted),
            ["home/prompt-unreadable:agora/PROTOCOL.md"]
        );
        assert_eq!(
            fs::read_to_string(root.join(PROTOCOL_FILE)).unwrap(),
            PROTOCOL
        );
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
        // Past the threshold too: nothing is archived over a damaged ledger.
        let past = ledger((0..9_001).map(|n| task(n, true, 2_000)).collect());
        let refused = booted.agora.write_tasks(&past).unwrap_err();
        assert!(matches!(refused, StoreError::Damaged(_)), "{refused:?}");
        assert!(!root.join(ArchivedTasks::FILE).exists());
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_reconcile_that_cannot_commit_is_reported_and_the_boot_goes_on() {
        let dir = scratch("cannot-reconcile");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join("agents/mason/outbox/m1.json"), "{}\n");
        let booted = Agora::open(&root, &prompts(&root), failing()).unwrap();
        assert_eq!(causes(&booted), ["agora/commit:reconcile"]);
        assert_eq!(booted.conditions[0].detail, "git is unwell");
        assert_eq!(
            subjects(&root),
            [SEED_SUBJECT],
            "the boot wrote nothing to commit"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_seed_that_cannot_commit_is_reported_and_stays_on_disk() {
        let dir = scratch("cannot-seed");
        let root = dir.join("agora");
        let booted = Agora::open(&root, &prompts(&root), failing()).unwrap();
        assert_eq!(causes(&booted), ["agora/commit:seed"]);
        assert_eq!(booted.agora.registry(), Loaded::Present(Registry::empty()));
        assert_eq!(
            booted.agora.committer().head(),
            None,
            "nothing is in history"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// R1.4a review (2026-10-20): when the reconcile failed, the seed commit
    /// swept the leftovers in under "seed the Agora".
    #[test]
    fn when_the_reconcile_fails_the_one_commit_names_the_leftovers_too() {
        let dir = scratch("mixed");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join("agents/mason/outbox/m1.json"), "{}\n");
        fs::remove_file(root.join("registry.json")).unwrap();
        let booted = Agora::open(&root, &prompts(&root), failing_once()).unwrap();
        assert_eq!(causes(&booted), ["agora/commit:reconcile"]);
        assert_eq!(
            subjects(&root),
            [RECONCILE_AND_SEED_SUBJECT, SEED_SUBJECT],
            "never a seed holding crash leftovers"
        );
        assert!(tracked(&root, "agents/mason/outbox/m1.json"));
        assert!(RECONCILE_AND_SEED_SUBJECT.starts_with(RECONCILE_SUBJECT));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn when_the_reconcile_fails_a_rendered_ignore_file_is_still_committed() {
        let dir = scratch("mixed-ignore");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join(GITIGNORE_FILE), "a hand edit\n");
        commit_by_hand(&root, "a hand edit");
        let booted = Agora::open(&root, &prompts(&root), failing_once()).unwrap();
        assert_eq!(causes(&booted), ["agora/commit:reconcile"]);
        assert_eq!(
            subjects(&root),
            [RECONCILE_AND_SEED_SUBJECT, "a hand edit", SEED_SUBJECT]
        );
        assert_eq!(
            git(&root, &["show", &format!("HEAD:{GITIGNORE_FILE}")]),
            GITIGNORE
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_held_agora_is_refused_in_the_committers_words() {
        let dir = scratch("held");
        let root = dir.join("agora");
        let first = open(&root);
        let refused = Agora::open(&root, &prompts(&root), Hooks::none()).unwrap_err();
        assert!(matches!(refused, OpenError::Held(_)), "{refused:?}");
        assert!(
            refused
                .to_string()
                .contains("another committer holds this Agora"),
            "{refused}"
        );
        drop(first);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// R1.4a review (2026-10-20): a file the boot could not write stopped the
    /// boot, though only the committer's refusal is fatal (SDD-RUST §10).
    #[test]
    fn a_file_the_boot_cannot_write_is_reported_and_the_boot_goes_on() {
        let dir = scratch("cannot-render");
        let root = dir.join("agora");
        drop(open(&root));
        // A directory where the protocol goes: the render cannot replace it.
        fs::remove_file(root.join(PROTOCOL_FILE)).unwrap();
        fs::create_dir_all(root.join(PROTOCOL_FILE).join("inside")).unwrap();
        let booted = open(&root);
        assert_eq!(causes(&booted), ["agora/render:PROTOCOL.md"]);
        let detail = &booted.conditions[0].detail;
        assert!(
            detail.starts_with("PROTOCOL.md could not be written ("),
            "{detail}"
        );
        assert!(detail.ends_with(RENDER_LATER), "{detail}");
        assert_eq!(booted.agora.registry(), Loaded::Present(Registry::empty()));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_roster_and_the_ledger_round_trip_through_their_stores() {
        let dir = scratch("round-trip");
        let root = dir.join("agora");
        let booted = open(&root);
        let agora = &booted.agora;
        agora.write_registry(&Registry::empty()).unwrap();
        let tasks = ledger(vec![task(1, true, 2_000), task(2, false, 1_000)]);
        assert_eq!(
            agora.write_tasks(&tasks).unwrap(),
            Retained {
                archived: 0,
                kept: tasks.clone()
            }
        );
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
        assert_eq!(retained.archived, 8_001);
        assert_eq!(retained.kept.tasks.len(), 1_000);
        let Loaded::Present(kept) = booted.agora.tasks() else {
            panic!("the trimmed ledger did not read back");
        };
        assert_eq!(kept, retained.kept, "what was written is what is returned");
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
        assert_eq!(booted.agora.write_tasks(&grown).unwrap().archived, 8_001);
        let archive = fs::read_to_string(root.join(ArchivedTasks::FILE)).unwrap();
        assert_eq!(archive.lines().count(), 2);
        assert!(archive.starts_with(lines[0]), "append-only");
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// R1.4a review D1 (2026-10-20): only the kept half was validated, so a
    /// task the ledger would refuse could be archived.
    #[test]
    fn a_ledger_write_validates_the_whole_ledger_before_it_archives() {
        let dir = scratch("validate-first");
        let root = dir.join("agora");
        let booted = open(&root);
        let mut tasks: Vec<Task> = (0..9_001)
            .map(|n| task(n, true, 2_000 + i64::from(n)))
            .collect();
        // The oldest done task, which the split would archive, is invalid.
        tasks[0].title = String::new();
        let refused = booted.agora.write_tasks(&ledger(tasks)).unwrap_err();
        assert!(matches!(refused, StoreError::Invalid(_)), "{refused:?}");
        assert!(refused.to_string().contains(".title"), "{refused}");
        assert!(!root.join(ArchivedTasks::FILE).exists());
        assert_eq!(booted.agora.tasks(), Loaded::Present(Tasks::empty()));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_archive_that_cannot_be_written_leaves_the_ledger_as_it_was() {
        let dir = scratch("archive-blocked");
        let root = dir.join("agora");
        let booted = open(&root);
        fs::create_dir_all(root.join(ArchivedTasks::FILE).join("inside")).unwrap();
        let past = ledger((0..9_001).map(|n| task(n, true, 2_000)).collect());
        let refused = booted.agora.write_tasks(&past).unwrap_err();
        assert!(matches!(refused, StoreError::Io(_)), "{refused:?}");
        assert_eq!(
            booted.agora.tasks(),
            Loaded::Present(Tasks::empty()),
            "the ledger is unchanged: every task is still in it"
        );
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
        fs::remove_file(root.join(BOARD_FILE)).unwrap();
        fs::create_dir_all(root.join(BOARD_FILE)).unwrap();
        assert!(
            agora.board().is_err(),
            "a board that cannot be read is an error, not empty"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }
}

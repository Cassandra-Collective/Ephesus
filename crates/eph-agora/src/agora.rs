//! `Agora`: the company's repository, opened in its boot order (ADR-0004;
//! SDD-RUST §2, §4.5, §10).
//!
//! Contract: [`Agora::open`] fails only when the committer refuses: another one
//! holds the Agora, or `git init` failed. Every later step that goes wrong
//! becomes a [`Condition`] in the degradation grammar, and the boot goes on
//! (invariant 7). The order:
//! 1. **The book, read** (R1.4b): the degradation channel replays the book's
//!    last [`REPLAY_LINES`] lines, as carried, and the boot notes a live file
//!    that ends mid-line (`agora/log-torn`) or a book it cannot read or open
//!    for writing. Nothing is written yet.
//! 2. **The committer** (`eph-git`): its lock, `git init` when there is no
//!    repository (after the lock files a killed `git init` left, which are
//!    reported), and an `index.lock` a killed git left. A repository made over
//!    a directory that already held files is reported, since whatever history
//!    it had is gone. The committer's hooks carry the caller's, the Agora's
//!    reporter (`agora/commit`) and its landing report (every `agora/commit`
//!    cause cleared, and what git ignores judged).
//! 3. **`.gitignore`, rendered** before anything is committed, so the reconcile
//!    never sweeps a message in flight or a killed write's temporary file into
//!    history.
//! 4. **The reconcile**: whatever a killed harness left is committed under its
//!    own subject, so the history never calls crash leftovers a seed. A
//!    repository this boot made over an empty directory has nothing to
//!    reconcile.
//! 5. **What the boot found so far, onto the degradation channel.** Only now,
//!    so no line this boot writes enters the reconcile, which is the last
//!    run's, and before the boot's own commit, whose landing clears
//!    `agora/commit:reconcile`.
//! 6. **`PROTOCOL.md`, rendered** from the home's override or the bundled copy
//!    (SDD-RUST §4.4, §4.6), so an improvement reaches every company that has
//!    already booted.
//! 7. **The seeds, only when absent**: an empty roster, an empty ledger, and
//!    the board (the home's override or the bundled text). A damaged roster or
//!    ledger is reported and left as evidence.
//! 8. **One commit, when this boot wrote anything**, the book included, named
//!    for what it holds: [`SEED_SUBJECT`] when it wrote files,
//!    [`RECORD_SUBJECT`] when it wrote only the book, each prefixed by the
//!    reconcile's ([`RECONCILE_AND_SEED_SUBJECT`],
//!    [`RECONCILE_AND_RECORD_SUBJECT`]) when the reconcile failed, since the
//!    leftovers are then in it. A boot that writes nothing commits nothing.
//! 9. **What git ignores, once more** when no batch landed during the boot
//!    ([`Committer::ignored`]); a landing already judged it.
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

use std::collections::HashSet;
use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use eph_fs::append::append_line;
use eph_fs::atomic::write_atomic;
use eph_fs::home::{Condition, prompt};
use eph_git::committer::{Committer, Hooks, Ignored, IndexLock, OpenError, STALE_AFTER};
use eph_schema::degradation::{Cause, Source};
use eph_schema::document::Versioned;
use eph_schema::error::SchemaError;
use eph_schema::id::TaskId;
use eph_schema::log::{LogKind, LogLine};
use eph_schema::registry::Registry;
use eph_schema::tasks::{ArchivedTasks, Task, Tasks, render_archived};
use eph_schema::time::Timestamp;

use crate::archive::{self, ArchiveError, Archived};
use crate::book::{AppendError, Appended, Book, Cursor, Draft, Read as BookRead};
use crate::degradations::{Degradations, REPLAY_LINES, agora as cause};
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
/// and nothing wider, so an agent's own `.notes.tmp` is still committed. The
/// third keeps a DIRECTORY of that name in history: `write_atomic` never makes
/// one, and the pattern alone hid a directory and everything in it (measured,
/// git 2.53; DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, third
/// round*). [`harness_ignores`] knows the same two shapes.
pub const GITIGNORE: &str = concat!(
    "# Written by the harness at every boot (SDD-RUST §2); an edit here is replaced.\n",
    "# A message in flight is a transient state the book of record already logs.\n",
    "agents/*/inbox/.inflight/\n",
    "# A write killed before its rename leaves .<name>.<8 hex>.tmp beside its file (eph-fs).\n",
    ".*.[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f].tmp\n",
    "# That name is a file's; a directory of the same name is work, and stays in history.\n",
    "!.*.[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f].tmp/\n",
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

/// The subject when the boot wrote only the book of record: the conditions it
/// found, on the degradation channel.
pub const RECORD_SUBJECT: &str = "record what the boot found";

/// [`RECORD_SUBJECT`] when the reconcile failed, so it names the leftovers.
pub const RECONCILE_AND_RECORD_SUBJECT: &str =
    "reconcile uncommitted work after restart, and record what the boot found";

/// A read of the book from a cursor that takes this long or more is reported
/// as `agora/log-size`, with the bytes it parsed and its record count (today's
/// figure and cause, `src/main/agora.ts`'s `DEFAULT_SLOW_READ_MS`;
/// DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, fourth round*,
/// item 4).
pub const SLOW_READ: std::time::Duration = std::time::Duration::from_millis(50);

/// The causes every boot checks again, so one that a boot does not find is
/// cleared, and its clear committed with the boot (DECISIONS-LOG 2026-10-25,
/// *R1.4b — Architect decisions, fourth round*, item 3). A cause the boot
/// checks only sometimes, such as the board's prompt override, read only when
/// the board is seeded, is not among them.
pub const BOOT_CHECKED: [&str; 14] = [
    "agora/log-torn",
    "agora/log-unreadable",
    "agora/log-unwritable",
    "agora/index-lock",
    "agora/init-lock",
    "agora/history-lost",
    "agora/render:.gitignore",
    "agora/render:PROTOCOL.md",
    "agora/seed:registry.json",
    "agora/seed:tasks.json",
    "agora/seed:board.md",
    "agora/damaged:registry.json",
    "agora/damaged:tasks.json",
    "home/prompt-unreadable:agora/PROTOCOL.md",
];

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
    /// What the boot found wrong. Each is already on the degradation channel,
    /// so a caller reports none of them again.
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
    book: Arc<Book>,
    degradations: Arc<Degradations>,
    slow_read: std::time::Duration,
}

impl Agora {
    /// Opens the Agora at `root` in its boot order, with the home's prompt
    /// overrides in `prompts`. `hooks` are the caller's: the Agora adds its own
    /// reporter and landing report to them. See the module contract.
    pub fn open(root: &Path, prompts: &Path, hooks: Hooks) -> Result<Booted, OpenError> {
        let held = holds_files(root);
        let book = Arc::new(Book::open(root));
        let degradations = Arc::new(Degradations::new(Arc::clone(&book)));
        // What the last run left, read before this boot writes anything.
        let mut conditions = book_conditions(&book, &degradations);
        let landed = Arc::new(AtomicBool::new(false));
        let opened = Committer::open(root, channel_hooks(hooks, &degradations, &landed))?;
        let agora = Agora {
            root: opened.committer.root().to_path_buf(),
            committer: opened.committer,
            book,
            degradations,
            slow_read: SLOW_READ,
        };
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
        // What the boot found so far reaches the channel only now, so no line
        // this boot writes is swept into the reconcile, which is the last
        // run's; and before the boot's commit, whose landing clears
        // `agora/commit:reconcile` (DECISIONS-LOG 2026-10-25, *R1.4b —
        // Architect decisions, second round*, item 4).
        let mut reported = agora.report_from(&conditions, 0);
        let protocol = prompt(prompts, PROTOCOL_PROMPT, PROTOCOL);
        conditions.extend(protocol.condition);
        let rendered = agora.render(PROTOCOL_FILE, &protocol.text, &mut conditions);
        let seeded = agora.seed(prompts, &mut conditions);
        reported = agora.report_from(&conditions, reported);
        // What an earlier boot found and this one checked again and did not
        // find no longer holds; its clear rides in the boot's commit.
        let found: Vec<&Cause> = conditions
            .iter()
            .map(|condition| &condition.cause)
            .collect();
        let resolved: Vec<Cause> = BOOT_CHECKED
            .iter()
            .filter_map(|text| text.parse::<Cause>().ok())
            .filter(|checked| !found.contains(&checked))
            .collect();
        agora
            .degradations
            .clear_where(|held| resolved.contains(held));
        // A `.gitignore` the reconcile already committed costs one commit that
        // lands with nothing staged. A boot that wrote only degradation lines
        // commits them too (SDD-RUST §10: one commit when anything was written).
        let files = ignore || rendered || seeded;
        if files || agora.book.appends() > 0 {
            let subject = match (reconcile_failed, files) {
                (false, true) => SEED_SUBJECT,
                (true, true) => RECONCILE_AND_SEED_SUBJECT,
                (false, false) => RECORD_SUBJECT,
                (true, false) => RECONCILE_AND_RECORD_SUBJECT,
            };
            if let Err(failure) = agora.committer.commit(subject).wait() {
                conditions.push(condition("commit", Some("seed"), failure.reason));
            }
        }
        // A boot with no landing still checks what git ignores, once.
        if !landed.load(Ordering::SeqCst) {
            judge_ignored(&agora.degradations, &agora.committer.ignored());
        }
        agora.report_from(&conditions, reported);
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

    /// The degradation channel: everything the boot found, every commit that
    /// nobody waited for and failed, and what an agent's ignore file hides.
    pub fn degradations(&self) -> &Degradations {
        &self.degradations
    }

    /// Appends `draft` to the book of record. A rotation the book had to skip
    /// is reported on the channel (invariant 7); the line still lands. A
    /// degradation line is refused: the channel is the one writer of that kind
    /// (DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, fourth round*,
    /// item 5). Until a later phase defines another kind's events, no line can
    /// be written here.
    pub fn append(&self, draft: Draft) -> Result<Appended, AppendError> {
        if draft.kind == LogKind::Degradation {
            return Err(AppendError::Invalid(SchemaError::invalid(
                LogLine::FILE,
                "kind",
                "a degradation line is the degradation channel's to write (Agora::degradations), so its ring and its ladder see every one",
            )));
        }
        self.degradations.append(draft)
    }

    /// A channel that receives every line appended from now on, in file
    /// order, after it is on disk. Dropping it unsubscribes.
    pub fn subscribe(&self) -> Receiver<LogLine> {
        self.book.subscribe()
    }

    /// Every line of the book after `cursor`. A read that takes
    /// [`SLOW_READ`] or more is reported with its size, so what reading the
    /// book costs is on the record.
    pub fn since(&self, cursor: Cursor) -> io::Result<BookRead> {
        let started = Instant::now();
        let read = self.book.since(cursor)?;
        let took = started.elapsed();
        if took >= self.slow_read {
            let bytes = read.cursor.offset().saturating_sub(cursor.offset());
            self.degradations.report(
                cause("log-size"),
                &format!(
                    "reading the book of record took {} ms for {} records over {bytes} bytes",
                    took.as_millis(),
                    read.entries.len()
                ),
            );
        }
        Ok(read)
    }

    /// The book's last `n` records, oldest first, and the cursor after them.
    pub fn tail(&self, n: usize) -> io::Result<BookRead> {
        self.book.tail(n)
    }

    /// Every archived task the ledger does not hold, the newest archive line
    /// winning a repeat (DECISIONS-LOG 2026-10-25, *R1.4b — Architect
    /// decisions, second round*, items 3 and 6). A task in both is the
    /// ledger's: a write cut short between the two files leaves one there.
    pub fn archived(&self) -> Result<Archived, ArchiveError> {
        let held: HashSet<TaskId> = match self.tasks() {
            Loaded::Present(ledger) => ledger.tasks.into_iter().map(|task| task.id).collect(),
            Loaded::Absent => HashSet::new(),
            Loaded::Damaged(damage) => return Err(ArchiveError::Ledger(damage)),
        };
        let bytes = match fs::read(self.root.join(ArchivedTasks::FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Archived::default()),
            Err(error) => return Err(ArchiveError::Io(error)),
        };
        Ok(archive::read(&bytes, &held))
    }

    /// Reports `conditions[from..]` on the channel, and says how far it got.
    fn report_from(&self, conditions: &[Condition], from: usize) -> usize {
        for condition in conditions.iter().skip(from) {
            self.degradations.report_condition(condition);
        }
        conditions.len()
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

/// Replays what was true when the last run stopped, and returns what the book
/// itself says about that run: a torn live file (DECISIONS-LOG 2026-10-25,
/// *R1.4b — Architect decisions, second round*, item 5), a book that cannot be
/// read or written. Writes nothing. The kill tests ask it what a reopen after
/// a real kill reports (GYM-014).
pub(crate) fn book_conditions(book: &Book, degradations: &Degradations) -> Vec<Condition> {
    let mut conditions = Vec::new();
    match book.tail(REPLAY_LINES) {
        Ok(tail) => degradations.replay(&tail.entries),
        Err(error) => conditions.push(condition(
            "log-unreadable",
            None,
            format!(
                "the book of record could not be read ({error}), so what was degraded when the harness last stopped is not shown"
            ),
        )),
    }
    // A live file that cannot be read for its last byte cannot be read by the
    // replay either, so that failure is already `agora/log-unreadable`.
    if book.ends_mid_line().unwrap_or(false) {
        conditions.push(condition(
            "log-torn",
            None,
            "the book of record's last line is torn: the last run was killed while it appended. The line is left on disk as evidence, and every line before it reads".to_string(),
        ));
    }
    if let Err(error) = book.writable() {
        conditions.push(condition(
            "log-unwritable",
            None,
            format!(
                "the book of record cannot be opened for writing ({error}); each append tries again, and the channel holds what it cannot write"
            ),
        ));
    }
    conditions
}

/// The caller's hooks, with the Agora's reporter and landing report added: a
/// failure nobody waited for raises `agora/commit`, and a landing clears every
/// `agora/commit` cause and checks what git ignores (DECISIONS-LOG 2026-10-25,
/// *R1.4b — Architect decisions, second round*, items 1 and 4). Neither queues
/// a commit, as the committer's contract requires.
fn channel_hooks(
    hooks: Hooks,
    degradations: &Arc<Degradations>,
    landed: &Arc<AtomicBool>,
) -> Hooks {
    let failed = Arc::clone(degradations);
    let landing = Arc::clone(degradations);
    let seen = Arc::clone(landed);
    hooks
        .reporting_to(move |failure| {
            failed.report(
                cause("commit"),
                &format!(
                    "\"{}\" was not committed ({}); the files are on disk, and the next commit that lands takes them",
                    failure.subject, failure.reason
                ),
            );
        })
        .on_landing(move |landed| {
            seen.store(true, Ordering::SeqCst);
            landing.clear_where(|held| held.source == Source::Agora && held.slug == "commit");
            judge_ignored(&landing, &landed.ignored);
        })
}

/// Names what git ignores that the harness did not make, as `agora/hidden-work`,
/// and clears it once nothing is; a listing git could not make is
/// `agora/ignore-check`.
fn judge_ignored(degradations: &Degradations, ignored: &Result<Ignored, String>) {
    let check = cause("ignore-check");
    let hidden = cause("hidden-work");
    let ignored = match ignored {
        Ok(ignored) => ignored,
        Err(reason) => {
            degradations.report(
                check,
                &format!(
                    "what git ignores in the Agora could not be listed ({reason}), so work an agent's own ignore file hides would go unnamed"
                ),
            );
            return;
        }
    };
    degradations.clear(&check);
    // A nested repository is listed by git as an ignored directory too, since
    // the committer keeps it out through the exclude file: name it once, as a
    // repository.
    let nested: Vec<&str> = ignored.nested.iter().map(String::as_str).collect();
    let theirs: Vec<&str> = ignored
        .entries
        .iter()
        .map(String::as_str)
        .filter(|entry| !harness_ignores(entry))
        .filter(|entry| {
            !ignored
                .nested
                .iter()
                .any(|path| entry.strip_suffix('/') == Some(path))
        })
        .collect();
    if theirs.is_empty() && nested.is_empty() {
        degradations.clear(&hidden);
    } else {
        degradations.report(hidden, &hidden_detail(&theirs, &nested));
    }
}

/// Whether an ignored entry, as the batch's status lists it, is one of the two
/// shapes the harness makes and its `.gitignore` names (DECISIONS-LOG
/// 2026-10-25, *R1.4b — Architect decisions, third round*, item 2): exactly a
/// message-in-flight directory, `agents/<id>/inbox/.inflight/`, or a FILE named
/// as `write_atomic` names its temporaries, `.<name>.<8 lowercase hex>.tmp`.
pub fn harness_ignores(entry: &str) -> bool {
    if let Some(directory) = entry.strip_suffix('/') {
        let parts: Vec<&str> = directory.split('/').collect();
        return matches!(parts.as_slice(), ["agents", id, "inbox", ".inflight"] if !id.is_empty());
    }
    is_temporary(entry.rsplit('/').next().unwrap_or(entry))
}

/// Whether `name` is `.<name>.<8 lowercase hex>.tmp`, with a name.
fn is_temporary(name: &str) -> bool {
    let Some(inner) = name
        .strip_prefix('.')
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((target, hex)) = inner.rsplit_once('.') else {
        return false;
    };
    !target.is_empty()
        && hex.len() == 8
        && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The detail of `agora/hidden-work`, naming the paths and as many of them as
/// fit.
fn hidden_detail(paths: &[&str], nested: &[&str]) -> String {
    let mut parts = Vec::new();
    if !paths.is_empty() {
        parts.push(format!(
            "git ignores {} path(s) the harness did not make, so they are missing from the Agora's history: {}. An ignore file an agent wrote hides them (a .gitignore under the Agora, or .git/info/exclude); remove it, and the next commit takes them",
            paths.len(),
            named(paths, 900)
        ));
    }
    if !nested.is_empty() {
        parts.push(format!(
            "{} git repository(ies) nested in the Agora are kept out of its history, so the rest of its work can land: {}. Remove each one's .git, or move it out of the Agora, and the next commit takes its files",
            nested.len(),
            named(nested, 500)
        ));
    }
    parts.join(". ")
}

/// `paths` joined, as many as fit in `room` characters, the rest counted.
/// One path is always named.
fn named(paths: &[&str], room: usize) -> String {
    let mut named = Vec::new();
    let mut used = 0;
    for path in paths {
        if used + path.len() > room && !named.is_empty() {
            break;
        }
        used += path.len() + 2;
        named.push(*path);
    }
    let more = paths.len() - named.len();
    let mut list = named.join(", ");
    if more > 0 {
        list.push_str(&format!(", and {more} more"));
    }
    list
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
    use eph_schema::degradation::{Degradation, DegradationEvent};
    use eph_schema::document::parse;
    use eph_schema::registry::{Agent, EngineId, HireRef};
    use eph_schema::tasks::TaskStatus;

    use super::*;
    use crate::book::tests::draft;
    use crate::book::{ARCHIVE_DIR, LOG_FILE};
    use crate::degradations::Freshness;
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
    fn a_boot_that_wrote_nothing_runs_one_batch_the_reconcile() {
        let dir = scratch("one-batch");
        let root = dir.join("agora");
        drop(open(&root));
        let batches = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&batches);
        let hooks = Hooks::none().on_landing(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let booted = Agora::open(&root, &prompts(&root), hooks).unwrap();
        assert!(booted.conditions.is_empty(), "{:?}", booted.conditions);
        assert_eq!(
            batches.load(Ordering::SeqCst),
            1,
            "no boot commit is queued when neither a file nor the book was written"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_list_of_names_holds_what_fits_in_its_room_and_counts_the_rest() {
        let ten = ["aaaaaaaaaa"; 5];
        // Each name costs its length and a separator: 12, 24, and the third
        // ends at exactly 34, so it fits; the fourth does not.
        assert_eq!(
            named(&ten, 34),
            "aaaaaaaaaa, aaaaaaaaaa, aaaaaaaaaa, and 2 more"
        );
        assert_eq!(named(&ten, 33), "aaaaaaaaaa, aaaaaaaaaa, and 3 more");
        assert_eq!(
            named(&ten, 0),
            "aaaaaaaaaa, and 4 more",
            "one is always named"
        );
        assert_eq!(named(&ten[..1], 1_000), "aaaaaaaaaa");
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
        // Since R1.4b the boot writes what it found to the book, so it has a
        // line to commit, and that commit meets the same unwell git.
        assert_eq!(
            causes(&booted),
            ["agora/commit:reconcile", "agora/commit:seed"]
        );
        assert_eq!(booted.conditions[0].detail, "git is unwell");
        assert_eq!(
            subjects(&root),
            [SEED_SUBJECT],
            "nothing this boot wrote reached history"
        );
        let held = booted.agora.degradations().list();
        assert_eq!(
            held.iter()
                .map(|entry| entry.cause.to_string())
                .collect::<Vec<_>>(),
            ["agora/commit:reconcile", "agora/commit:seed"],
            "both are on the channel, and no commit landed to clear them"
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

    // ---- R1.4b: the book of record and the degradation channel --------

    fn held(agora: &Agora) -> Vec<String> {
        agora
            .degradations()
            .list()
            .iter()
            .map(|entry| entry.cause.to_string())
            .collect()
    }

    fn entry(agora: &Agora, text: &str) -> crate::degradations::Entry {
        let cause: Cause = text.parse().unwrap();
        agora
            .degradations()
            .list()
            .into_iter()
            .find(|entry| entry.cause == cause)
            .unwrap_or_else(|| panic!("{text} is not on the channel: {:?}", held(agora)))
    }

    fn rows(agora: &Agora) -> Vec<Degradation> {
        agora
            .since(Cursor::START)
            .unwrap()
            .entries
            .iter()
            .filter_map(|line| Degradation::from_line(line).ok())
            .collect()
    }

    fn has(agora: &Agora, text: &str) -> bool {
        agora.degradations().has(&text.parse().unwrap())
    }

    /// Hooks that fault every `BeforeStage` while `armed` holds.
    fn armed(armed: &Arc<AtomicBool>) -> Hooks {
        let armed = Arc::clone(armed);
        Hooks::none().with_faults(move |point| {
            if point == FaultPoint::BeforeStage && armed.load(Ordering::SeqCst) {
                return Err("git is unwell".to_string());
            }
            Ok(())
        })
    }

    #[test]
    fn a_boot_replays_what_held_when_the_last_run_stopped_as_carried() {
        let dir = scratch("replay");
        let root = dir.join("agora");
        let booted = open(&root);
        booted
            .agora
            .degradations()
            .report("hermes/stranded:m1".parse().unwrap(), "a message waits");
        drop(booted);
        let again = open(&root);
        let carried = entry(&again.agora, "hermes/stranded:m1");
        assert_eq!(carried.freshness, Freshness::Carried);
        assert_eq!(carried.detail, "a message waits");
        assert!(
            again.conditions.is_empty(),
            "a replay is not this boot's finding"
        );
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_book_is_reported_at_boot_and_its_line_left_as_evidence() {
        let dir = scratch("torn");
        let root = dir.join("agora");
        drop(open(&root));
        let torn = "{\"schemaVersion\":1,\"seq\":1,\"ts\"";
        write(&root.join(LOG_FILE), torn);
        let booted = open(&root);
        assert_eq!(causes(&booted), ["agora/log-torn"]);
        assert!(
            booted.conditions[0]
                .detail
                .contains("killed while it appended"),
            "{}",
            booted.conditions[0].detail
        );
        assert_eq!(
            subjects(&root),
            [RECORD_SUBJECT, RECONCILE_SUBJECT, SEED_SUBJECT],
            "the last run's torn bytes are its leftovers; this boot's line is its own commit"
        );
        assert_eq!(
            git(&root, &["show", &format!("HEAD~1:{LOG_FILE}")]),
            torn,
            "the reconcile holds the last run's bytes and nothing this boot wrote"
        );
        let text = fs::read_to_string(root.join(LOG_FILE)).unwrap();
        assert!(text.starts_with(&format!("{torn}\n")), "{text}");
        let read = booted.agora.since(Cursor::START).unwrap();
        assert_eq!(
            read.damaged, 1,
            "the torn line is counted once it is not the last"
        );
        assert_eq!(rows(&booted.agora)[0].cause.to_string(), "agora/log-torn");
        drop(booted);
        let again = open(&root);
        assert!(
            again.conditions.is_empty(),
            "a book that ends whole is not torn: {:?}",
            again.conditions
        );
        // DECISIONS-LOG 2026-10-25, fourth round, item 3: checked again and
        // not found, so cleared rather than carried, and the clear committed.
        assert!(
            !has(&again.agora, "agora/log-torn"),
            "{:?}",
            held(&again.agora)
        );
        let cleared = rows(&again.agora).pop().unwrap();
        assert_eq!(cleared.cause.to_string(), "agora/log-torn");
        assert_eq!(cleared.event(), DegradationEvent::Cleared);
        assert_eq!(subjects(&root)[0], RECORD_SUBJECT);
        assert!(!again.agora.committer().is_dirty());
        drop(again);
        let third = open(&root);
        assert_eq!(
            subjects(&root)[0],
            RECORD_SUBJECT,
            "nothing left to clear: the third boot commits nothing"
        );
        assert_eq!(subjects(&root).len(), 4);
        drop(third);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_clears_only_the_boot_causes_it_checked_again_and_did_not_find() {
        let dir = scratch("boot-clears");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join(".git").join("index.lock"), "");
        let booted = open(&root);
        assert_eq!(causes(&booted), ["agora/index-lock"]);
        booted
            .agora
            .degradations()
            .report("hermes/stranded:m1".parse().unwrap(), "not a boot cause");
        drop(booted);
        let again = open(&root);
        let still: Vec<String> = held(&again.agora);
        assert_eq!(
            still,
            ["hermes/stranded:m1"],
            "the index lock is gone and cleared; a cause no boot checks stays carried"
        );
        assert_eq!(
            entry(&again.agora, "hermes/stranded:m1").freshness,
            Freshness::Carried
        );
        for text in BOOT_CHECKED {
            assert!(text.parse::<Cause>().is_ok(), "{text}");
        }
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_replays_only_the_books_last_four_hundred_lines() {
        let dir = scratch("replay-bound");
        let root = dir.join("agora");
        let booted = open(&root);
        let channel = booted.agora.degradations();
        channel.report("hermes/stranded:old".parse().unwrap(), "out of reach");
        // Exactly the replay's 400 lines after it: a raise and a clear each.
        let churn: Cause = "hermes/stranded:churn".parse().unwrap();
        for _ in 0..REPLAY_LINES / 2 {
            channel.report(churn.clone(), "up");
            channel.clear(&churn);
        }
        drop(booted);
        let again = open(&root);
        assert!(
            !has(&again.agora, "hermes/stranded:old"),
            "older than the last 400 lines: {:?}",
            held(&again.agora)
        );
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_commit_that_fails_clears_at_the_next_commit_that_lands() {
        let dir = scratch("seed-clears");
        let root = dir.join("agora");
        let gate = Arc::new(AtomicBool::new(true));
        let booted = Agora::open(&root, &prompts(&root), armed(&gate)).unwrap();
        assert_eq!(causes(&booted), ["agora/commit:seed"]);
        assert!(has(&booted.agora, "agora/commit:seed"));
        gate.store(false, Ordering::SeqCst);
        booted.agora.committer().commit("later").wait().unwrap();
        assert!(!has(&booted.agora, "agora/commit:seed"), "late, not lost");
        assert!(tracked(&root, "registry.json"));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_book_the_boot_cannot_read_or_write_is_reported_and_the_boot_goes_on() {
        let dir = scratch("book-blocked");
        let root = dir.join("agora");
        drop(open(&root));
        fs::create_dir_all(root.join(LOG_FILE).join("inside")).unwrap();
        let booted = open(&root);
        assert_eq!(booted.agora.root(), root);
        assert_eq!(
            causes(&booted),
            ["agora/log-unreadable", "agora/log-unwritable"]
        );
        assert!(
            booted.conditions[0]
                .detail
                .contains("what was degraded when the harness last stopped is not shown"),
            "{}",
            booted.conditions[0].detail
        );
        assert!(
            booted.conditions[1]
                .detail
                .contains("each append tries again"),
            "{}",
            booted.conditions[1].detail
        );
        assert_eq!(
            held(&booted.agora),
            ["agora/log-unreadable", "agora/log-unwritable"],
            "the channel holds what the book could not take"
        );
        assert_eq!(
            subjects(&root),
            [SEED_SUBJECT],
            "nothing was written to commit"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_boots_conditions_stay_out_of_the_reconcile_and_land_in_one_commit() {
        let dir = scratch("conditions-commit");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join(".git").join("index.lock"), "");
        write(&root.join("agents/mason/outbox/m1.json"), "{}\n");
        let booted = open(&root);
        assert_eq!(causes(&booted), ["agora/index-lock"]);
        assert_eq!(
            subjects(&root),
            [RECORD_SUBJECT, RECONCILE_SUBJECT, SEED_SUBJECT]
        );
        let reconciled = git(&root, &["show", "--name-only", "--format=", "HEAD~1"]);
        assert!(
            reconciled.contains("agents/mason/outbox/m1.json"),
            "{reconciled}"
        );
        assert!(!reconciled.contains(LOG_FILE), "{reconciled}");
        let recorded = git(&root, &["show", "--name-only", "--format=", "HEAD"]);
        assert_eq!(recorded.trim(), LOG_FILE);
        assert_eq!(held(&booted.agora), ["agora/index-lock"]);
        assert!(!booted.agora.committer().is_dirty());
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_agora_commit_cause_clears_at_the_next_commit_that_lands() {
        let dir = scratch("clears");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join("agents/mason/outbox/m1.json"), "{}\n");
        // A slow landing report of the caller's runs before the Agora's: were
        // the reports made after the ticket is answered, the clear below would
        // be missed every time, not only when a thread loses a race.
        let slow = failing_once()
            .on_landing(|_| std::thread::sleep(std::time::Duration::from_millis(200)));
        let booted = Agora::open(&root, &prompts(&root), slow).unwrap();
        assert_eq!(causes(&booted), ["agora/commit:reconcile"]);
        assert_eq!(
            subjects(&root),
            [RECONCILE_AND_RECORD_SUBJECT, SEED_SUBJECT],
            "the one commit holds the leftovers and the line, and names both"
        );
        assert!(tracked(&root, "agents/mason/outbox/m1.json"));
        assert!(
            !has(&booted.agora, "agora/commit:reconcile"),
            "the boot's own commit landed and cleared it"
        );
        let cleared = rows(&booted.agora)
            .into_iter()
            .find(|row| row.event() == DegradationEvent::Cleared)
            .unwrap();
        assert_eq!(cleared.cause.to_string(), "agora/commit:reconcile");
        assert!(cleared.cleared_after.is_some());
        drop(booted);

        // A commit nobody waited for raises `agora/commit`, and a caller's own
        // reporter still hears it beside the Agora's.
        let gate = Arc::new(AtomicBool::new(false));
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        let hooks = armed(&gate).reporting_to(move |failure| {
            into.lock().unwrap().push(failure.subject.clone());
        });
        let booted = Agora::open(&root, &prompts(&root), hooks).unwrap();
        gate.store(true, Ordering::SeqCst);
        write(&root.join("agents/mason/outbox/m2.json"), "{}\n");
        booted.agora.committer().commit_soon("deliver m2");
        booted.agora.committer().drained();
        assert_eq!(*heard.lock().unwrap(), ["deliver m2"]);
        let late = entry(&booted.agora, "agora/commit");
        assert!(
            late.detail
                .contains("\"deliver m2\" was not committed (git is unwell)"),
            "{}",
            late.detail
        );
        assert!(
            late.detail
                .ends_with("the next commit that lands takes them"),
            "{}",
            late.detail
        );
        gate.store(false, Ordering::SeqCst);
        booted
            .agora
            .committer()
            .commit("deliver m3")
            .wait()
            .unwrap();
        assert!(!has(&booted.agora, "agora/commit"));
        assert!(
            tracked(&root, "agents/mason/outbox/m2.json"),
            "late, not lost"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn work_an_agents_own_ignore_file_hides_is_named_until_nothing_is() {
        let dir = scratch("hidden");
        let root = dir.join("agora");
        let booted = open(&root);
        let mason = root.join("agents/mason");
        write(&mason.join(".gitignore"), "secret/\n*.draft\n");
        write(&mason.join("secret/plan.md"), "hidden\n");
        write(&mason.join("notes.draft"), "hidden too\n");
        write(&mason.join("inbox/.inflight/m1.json"), "{}\n");
        write(&mason.join("outbox/.m2.json.0a1b2c3d.tmp"), "half");
        booted.agora.committer().commit("work").wait().unwrap();
        let hidden = entry(&booted.agora, "agora/hidden-work");
        assert!(
            hidden
                .detail
                .starts_with("git ignores 2 path(s) the harness did not make"),
            "{}",
            hidden.detail
        );
        assert!(
            hidden.detail.contains("agents/mason/notes.draft"),
            "{}",
            hidden.detail
        );
        assert!(
            hidden.detail.contains("agents/mason/secret/"),
            "{}",
            hidden.detail
        );
        assert!(
            !hidden.detail.contains(".inflight"),
            "the harness's own: {}",
            hidden.detail
        );
        assert!(
            !hidden.detail.contains(".tmp"),
            "the harness's own: {}",
            hidden.detail
        );
        booted
            .agora
            .committer()
            .commit("still hidden")
            .wait()
            .unwrap();
        assert_eq!(entry(&booted.agora, "agora/hidden-work").count, 2);
        fs::remove_file(mason.join(".gitignore")).unwrap();
        booted.agora.committer().commit("unhidden").wait().unwrap();
        assert!(!has(&booted.agora, "agora/hidden-work"));
        assert!(tracked(&root, "agents/mason/secret/plan.md"));
        assert!(tracked(&root, "agents/mason/notes.draft"));
        assert!(rows(&booted.agora).iter().any(|row| {
            row.cause.to_string() == "agora/hidden-work" && row.event() == DegradationEvent::Cleared
        }));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, third round*,
    /// item 1: the temporary pattern alone hid a directory of that name.
    #[test]
    fn a_directory_named_like_a_temporary_file_is_committed_and_the_file_is_not() {
        let dir = scratch("temp-dir");
        let root = dir.join("agora");
        let booted = open(&root);
        write(
            &root.join("agents/mason/.plan.0123abcd.tmp/work.md"),
            "work\n",
        );
        write(&root.join("agents/mason/.plan.json.0123abcd.tmp"), "half");
        booted.agora.committer().commit("work").wait().unwrap();
        assert!(tracked(&root, "agents/mason/.plan.0123abcd.tmp/work.md"));
        assert!(!tracked(&root, "agents/mason/.plan.json.0123abcd.tmp"));
        assert!(
            !has(&booted.agora, "agora/hidden-work"),
            "{:?}",
            held(&booted.agora)
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_harness_ignores_exactly_the_two_shapes_it_makes() {
        for ours in [
            "agents/mason/inbox/.inflight/",
            "agents/.x/inbox/.inflight/",
            ".registry.json.0a1b2c3d.tmp",
            "agents/mason/outbox/.m1.json.0123abcd.tmp",
            ".x.0123abcd.tmp",
        ] {
            assert!(harness_ignores(ours), "{ours}");
        }
        for theirs in [
            "agents//inbox/.inflight/",
            "agents/mason/inbox/.inflight",
            "agents/mason/INBOX/.INFLIGHT/",
            "agents/mason/x/inbox/.inflight/",
            "x/agents/mason/inbox/.inflight/",
            "agents/mason/inbox/.done/",
            ".plan.0123abcd.tmp/",
            "..0123abcd.tmp",
            ".x.0123ABCD.tmp",
            ".x.0123abc.tmp",
            ".x.0123abcde.tmp",
            ".x.0123abcg.tmp",
            "x.0123abcd.tmp",
            ".x.0123abcd.tmpx",
            ".x0123abcd.tmp",
            "agents/mason/secret/",
            "a.log",
            ".notes.tmp",
        ] {
            assert!(!harness_ignores(theirs), "{theirs}");
        }
    }

    #[test]
    fn a_long_list_of_hidden_paths_names_what_fits_and_counts_the_rest() {
        let paths: Vec<String> = (0..300)
            .map(|n| format!("agents/mason/secret-{n:03}/"))
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let nested: Vec<String> = (0..100).map(|n| format!("agents/a{n:03}/work")).collect();
        let nested: Vec<&str> = nested.iter().map(String::as_str).collect();
        let detail = hidden_detail(&refs, &nested);
        assert!(detail.starts_with("git ignores 300 path(s)"), "{detail}");
        assert!(detail.contains("agents/mason/secret-000/"));
        assert!(!detail.contains("agents/mason/secret-299/"));
        assert!(detail.contains("more."), "{detail}");
        assert!(
            detail.contains(". 100 git repository(ies) nested in the Agora are kept out"),
            "{detail}"
        );
        assert!(detail.contains("agents/a000/work"));
        assert!(!detail.contains("agents/a099/work"));
        assert!(detail.chars().count() <= crate::degradations::MAX_DETAIL);
        let one = hidden_detail(&["agents/mason/x/"], &[]);
        assert!(
            one.contains("missing from the Agora's history: agents/mason/x/. An"),
            "{one}"
        );
        assert!(!one.contains("nested"), "{one}");
        let only = hidden_detail(&[], &["agents/a1/work"]);
        assert!(
            only.starts_with("1 git repository(ies) nested in the Agora"),
            "{only}"
        );
        assert!(only.ends_with("the next commit takes its files"), "{only}");
        let huge = "y".repeat(3_000);
        assert!(
            hidden_detail(&[&huge], &[]).contains(&huge),
            "one path is always named"
        );
    }

    /// DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, fourth round*,
    /// item 1.
    #[test]
    fn a_repository_nested_in_the_agora_is_named_once_and_its_neighbours_land() {
        let dir = scratch("nested");
        let root = dir.join("agora");
        let booted = open(&root);
        let work = root.join("agents/mason/work");
        write(&work.join("plan.md"), "the plan\n");
        let made = Git::target()
            .without_machine_config()
            .run(&work, &["init", "-q", "-b", "main"]);
        assert!(made.ok(), "{made:?}");
        write(&root.join("agents/ada/outbox/m1.json"), "{}\n");
        booted.agora.committer().commit("work").wait().unwrap();
        assert!(
            tracked(&root, "agents/ada/outbox/m1.json"),
            "the rest of the company's work landed"
        );
        assert!(!tracked(&root, "agents/mason/work/plan.md"));
        let hidden = entry(&booted.agora, "agora/hidden-work");
        assert!(
            hidden.detail.starts_with(
                "1 git repository(ies) nested in the Agora are kept out of its history"
            ),
            "named as a repository, not as an ignored directory: {}",
            hidden.detail
        );
        assert!(
            hidden.detail.contains("agents/mason/work"),
            "{}",
            hidden.detail
        );
        fs::remove_dir_all(work.join(".git")).unwrap();
        booted.agora.committer().commit("unnested").wait().unwrap();
        assert!(tracked(&root, "agents/mason/work/plan.md"));
        assert!(!has(&booted.agora, "agora/hidden-work"));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_with_no_landing_still_checks_what_git_ignores() {
        let dir = scratch("no-landing");
        let root = dir.join("agora");
        drop(open(&root));
        write(&root.join("agents/mason/.gitignore"), "secret/\n");
        write(&root.join("agents/mason/secret/plan.md"), "hidden\n");
        let booted = Agora::open(&root, &prompts(&root), failing()).unwrap();
        assert!(causes(&booted).contains(&"agora/commit:reconcile".to_string()));
        assert!(
            entry(&booted.agora, "agora/hidden-work")
                .detail
                .contains("agents/mason/secret/"),
            "the boot asked, though no batch landed"
        );
        drop(booted);
        write(&root.join(".git").join("index"), "not an index");
        let booted = Agora::open(&root, &prompts(&root), failing()).unwrap();
        let check = entry(&booted.agora, "agora/ignore-check");
        assert!(
            check
                .detail
                .contains("could not be listed (git status failed"),
            "{}",
            check.detail
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_channel_is_the_one_writer_of_degradation_lines_and_publishes_them() {
        let dir = scratch("publish");
        let root = dir.join("agora");
        let booted = open(&root);
        let heard = booted.agora.subscribe();
        let refused = booted.agora.append(draft(1)).unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("the degradation channel's to write"),
            "{refused}"
        );
        let mut gym = draft(1);
        gym.kind = LogKind::Gym;
        let undefined = booted.agora.append(gym).unwrap_err();
        assert!(
            undefined.to_string().contains("not defined yet"),
            "{undefined}"
        );
        assert!(heard.try_recv().is_err(), "nothing was written");
        booted
            .agora
            .degradations()
            .report("hermes/stranded:m1".parse().unwrap(), "a message waits");
        let line = heard
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(line.kind, LogKind::Degradation);
        let tail = booted.agora.tail(1).unwrap();
        assert_eq!(tail.entries, [line]);
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_slow_read_of_the_book_is_reported_as_log_size_with_what_it_read() {
        let dir = scratch("slow-read");
        let root = dir.join("agora");
        let mut booted = open(&root);
        booted
            .agora
            .degradations()
            .report("hermes/stranded:m1".parse().unwrap(), "a message waits");
        booted.agora.since(Cursor::START).unwrap();
        assert!(
            !has(&booted.agora, "agora/log-size"),
            "a quick read is not news"
        );
        let len = fs::metadata(root.join(LOG_FILE)).unwrap().len();
        booted.agora.slow_read = std::time::Duration::ZERO;
        booted.agora.since(Cursor::START).unwrap();
        let slow = entry(&booted.agora, "agora/log-size");
        assert!(
            slow.detail.starts_with("reading the book of record took "),
            "{}",
            slow.detail
        );
        assert!(
            slow.detail
                .ends_with(&format!(" ms for 1 records over {len} bytes")),
            "the bytes that read parsed: {}",
            slow.detail
        );
        let end = booted.agora.since(Cursor::START).unwrap().cursor;
        booted.agora.since(end).unwrap();
        assert!(
            entry(&booted.agora, "agora/log-size")
                .detail
                .ends_with(" ms for 0 records over 0 bytes"),
            "a read from the end parsed nothing"
        );
        assert_eq!(SLOW_READ, std::time::Duration::from_millis(50));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rotation_the_book_must_skip_is_reported_through_the_agora() {
        let dir = scratch("rotation");
        let root = dir.join("agora");
        let booted = open(&root);
        booted
            .agora
            .degradations()
            .report("hermes/stranded:m1".parse().unwrap(), "a message waits");
        drop(booted);
        // Past four mebibytes, with its segment's name already taken.
        let mut bytes = fs::read(root.join(LOG_FILE)).unwrap();
        let junk = format!("{}\n", "x".repeat(1_023));
        while bytes.len() < 4 * 1024 * 1024 + 1 {
            bytes.extend_from_slice(junk.as_bytes());
        }
        write_atomic(&root.join(LOG_FILE), &bytes).unwrap();
        write(
            &root.join(ARCHIVE_DIR).join(eph_fs::append::segment_name(1)),
            "history\n",
        );
        let booted = open(&root);
        booted
            .agora
            .degradations()
            .report("hermes/stranded:m2".parse().unwrap(), "another waits");
        assert!(
            entry(&booted.agora, "agora/log-rotation")
                .detail
                .contains("would rewrite history")
        );
        assert!(
            fs::metadata(root.join(LOG_FILE)).unwrap().len() > 4 * 1024 * 1024,
            "the line landed in the live file all the same"
        );
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_archive_is_read_against_the_ledger() {
        let dir = scratch("archived");
        let root = dir.join("agora");
        let booted = open(&root);
        assert_eq!(
            booted.agora.archived().unwrap(),
            Archived::default(),
            "absent"
        );
        let tasks = ledger(
            (0..9_001)
                .map(|n| task(n, true, 2_000 + i64::from(n)))
                .collect(),
        );
        let retained = booted.agora.write_tasks(&tasks).unwrap();
        let archived = booted.agora.archived().unwrap();
        assert_eq!(archived.tasks.len(), retained.archived);
        assert_eq!(archived.tasks[0].title, "task 0");
        let kept: HashSet<TaskId> = retained
            .kept
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect();
        assert!(archived.tasks.iter().all(|task| !kept.contains(&task.id)));
        write(&root.join("tasks.json"), "{ evidence");
        let refused = booted.agora.archived().unwrap_err();
        assert!(matches!(refused, ArchiveError::Ledger(_)), "{refused:?}");
        assert!(
            refused.to_string().contains("which is damaged"),
            "{refused}"
        );
        fs::remove_file(root.join("tasks.json")).unwrap();
        fs::remove_file(root.join(ArchivedTasks::FILE)).unwrap();
        fs::create_dir_all(root.join(ArchivedTasks::FILE).join("inside")).unwrap();
        assert!(matches!(booted.agora.archived(), Err(ArchiveError::Io(_))));
        drop(booted);
        fs::remove_dir_all(&dir).unwrap();
    }
}

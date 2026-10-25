//! `Committer`: the Agora's single committer (ADR-0004; FR-4.1; invariant 4).
//!
//! Contract:
//! - **One committer per Agora, enforced.** [`Committer::open`] takes an OS lock
//!   on `.git/ephesus-committer.lock` before it does anything else, and holds it
//!   until the committer is dropped and its last git has finished. A second
//!   committer on the same Agora, in this process or another, is refused with
//!   [`OpenError::Held`], after [`LOCK_WAITS`]: on POSIX a lock released a
//!   moment ago can read as held while a process another thread forked has
//!   not yet exec'd. The OS releases the lock when the process dies, however
//!   it dies (DECISIONS-LOG 2026-10-20, *R1.3 — Architect decisions*).
//! - **A killed harness's `index.lock` is judged at open**, with the lock held
//!   and before any commit can run. Open waits while `.git/index.lock` exists,
//!   over the retry schedule, so a git still running for a killed harness can
//!   finish, and then removes one that outlived it ([`Opened::index_lock`]).
//! - **A `git init` a kill left unfinished is finished.** When `.git` has no
//!   `HEAD`, open waits over the same schedule while any other lock file sits
//!   directly in `.git` (a killed init's `config.lock`, say), removes those
//!   left, and runs `git init`, naming them in [`Opened::init_locks`]. Without
//!   that, the next `git init` refuses the lock, and so would every later open
//!   (DECISIONS-LOG 2026-10-25, *R1.4a — Architect decisions, `eph-git`*).
//! - **One queue.** One thread runs every git call on the Agora, so two can never
//!   run at once and `index.lock` contention cannot be self-inflicted.
//! - **Batching.** Commits enqueued while the thread is busy land together, as
//!   one commit whose message names every subject. Delivery is a rename,
//!   durability is a commit, and commit throughput stays off delivery's path.
//! - **Retries live here and nowhere else** (BUILD-PROMPT §7). A failed git
//!   call is retried, five attempts in all, over the fixed [`RETRY_WAITS`].
//!   Before each retry an `index.lock` is removed only if it is older than the
//!   runner's deadline, because no git of this committer's can still hold one
//!   that old. A younger one belongs to someone else, such as a `git status` in
//!   the Architect's shell, and is waited on, never removed.
//! - **A fault aborts its batch, with no retry.** A [`Hooks`] fault that refuses
//!   at a [`FaultPoint`] stands for the harness dying there (TEST-STRATEGY §3,
//!   S-BLACKOUT). Its reason is the batch's failure, word for word.
//! - **Reconcile after a restart.** [`Committer::reconcile`] commits whatever a
//!   killed harness left, under [`RECONCILE_SUBJECT`].
//! - **A failure loses no work and is never silent.** A commit that gives up
//!   leaves the files on disk, correct and not yet in history, and the next
//!   commit picks them up. A failure nobody waits for, from
//!   [`Committer::commit_soon`] or a dropped [`Ticket`], is reported through
//!   [`Hooks::reporting_to`] and kept ([`Committer::failures`], the latest
//!   [`MAX_RECORDED_FAILURES`]) (invariant 7).
//! - **A landing is reported too**, through [`Hooks::on_landing`], before any
//!   ticket of its batch is answered, so a caller that waited sees what the
//!   report did. Nothing else sees a [`Committer::commit_soon`] land, and the
//!   degradation channel clears `agora/commit` on it (DECISIONS-LOG 2026-10-25,
//!   *R1.4b — Architect decisions, second round*, item 4).
//! - **Each landing lists what git ignored**, from the status the batch already
//!   runs after staging (`--ignored=matching`), so the check costs no second
//!   walk of the tree (item 1 of the same entry). A directory an ignore pattern
//!   matched is listed as the directory, ending in `/`, not its files.
//!   [`Committer::ignored`] asks the same of a tree no batch has looked at. What
//!   the harness's own patterns ignore is the caller's to tell apart.
//! - **Dropping the committer lands its queue first**, then releases the lock.
//!   That can take as long as the queued gits do, each bounded by its deadline.
//!
//! The Agora's own files (`PROTOCOL.md`, the registry, the ledger, the board,
//! the book of record) are R1.4's. [`Opened::initialized`] tells that boot
//! whether this open made the repository.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io;
use std::iter;
use std::path::{self, Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eph_fs::atomic::write_atomic;
use eph_fs::lock::Exclusive;

use crate::runner::{Git, GitResult, TIMEOUT};

/// The committer's lock file, inside the Agora's `.git`, so no `add -A` sees it.
pub const LOCK_FILE: &str = "ephesus-committer.lock";

/// The file a batch's message is written to inside `.git`, and committed from
/// with `-F`: a message on the command line is capped by the OS (about 32,000
/// characters on Windows), and a large batch would never land.
pub const MESSAGE_FILE: &str = "EPHESUS_COMMIT_MSG";

/// The waits before each retry, in milliseconds: five attempts in all, 25 ms
/// doubling (today's figures), as a fixed schedule that cannot run forever.
pub const RETRY_WAITS: [u64; 4] = [25, 50, 100, 200];

/// How old an `index.lock` must be before a retry removes it: the runner's
/// deadline, past which every git of this committer's has been killed.
pub const STALE_AFTER: Duration = TIMEOUT;

/// The waits, in milliseconds, before open asks for the committer's lock again.
/// On POSIX a process another thread forks holds a copy of every open file
/// until it execs, the lock's included, so a lock released a moment ago can
/// still read as held (CI, Linux, 2026-10-20). A committer that is really
/// running still holds it after these 150 ms, and is refused.
pub const LOCK_WAITS: [u64; 4] = [10, 20, 40, 80];

/// How many failures nobody waited for are kept, oldest dropped first.
pub const MAX_RECORDED_FAILURES: usize = 50;

/// The subject a reconcile commits under.
pub const RECONCILE_SUBJECT: &str = "reconcile uncommitted work after restart";

/// The subject of a batch whose first subject is blank.
const BLANK_SUBJECT: &str = "agora update";

/// Where a test may stop a commit, as the harness dying there would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// Before `git add -A`.
    BeforeStage,
    /// After staging, before the commit: the S-BLACKOUT point.
    AfterStage,
    /// Just before `git commit`.
    BeforeCommit,
    /// After the commit, before its outcome is reported.
    AfterCommit,
}

type FaultHook = dyn Fn(FaultPoint) -> Result<(), String> + Send + Sync;
type ReportHook = dyn Fn(&CommitFailure) + Send + Sync;
type LandingHook = dyn Fn(&Landing) + Send + Sync;

/// What a caller hangs on the committer. Production passes a reporter and a
/// landing report, for the degradation channel (R1.4b); tests add faults. The
/// fault seam is in the production path on purpose, because a seam that exists
/// only under a mock proves nothing about the real order of the steps
/// (TEST-STRATEGY §3). Reporters add up: each one hung on is called, in the
/// order they were hung on, so a caller's reporter survives the Agora's.
#[derive(Clone)]
pub struct Hooks {
    faults: Option<Arc<FaultHook>>,
    report: Vec<Arc<ReportHook>>,
    landing: Vec<Arc<LandingHook>>,
}

impl Hooks {
    /// No faults and no reporter.
    pub fn none() -> Hooks {
        Hooks {
            faults: None,
            report: Vec::new(),
            landing: Vec::new(),
        }
    }

    /// The same hooks with faults decided by `hook` at every point: `Err(reason)`
    /// aborts that batch with `reason`.
    pub fn with_faults(
        mut self,
        hook: impl Fn(FaultPoint) -> Result<(), String> + Send + Sync + 'static,
    ) -> Hooks {
        self.faults = Some(Arc::new(hook));
        self
    }

    /// The same hooks also reporting every failure nobody waited for to `hook`,
    /// on the committer's thread. Contract: `hook` must not queue a commit. The
    /// failing path is git itself, and a reporter that commits would recurse.
    pub fn reporting_to(mut self, hook: impl Fn(&CommitFailure) + Send + Sync + 'static) -> Hooks {
        self.report.push(Arc::new(hook));
        self
    }

    /// The same hooks also telling `hook` of every batch that lands, on the
    /// committer's thread, before the batch's tickets are answered. Contract:
    /// `hook` must not queue a commit or wait on the committer, which is busy
    /// calling it.
    pub fn on_landing(mut self, hook: impl Fn(&Landing) + Send + Sync + 'static) -> Hooks {
        self.landing.push(Arc::new(hook));
        self
    }

    fn check(&self, point: FaultPoint) -> Result<(), String> {
        match &self.faults {
            Some(hook) => hook(point),
            None => Ok(()),
        }
    }
}

/// A batch that landed, or found nothing to land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    /// Every subject in the batch, in the order they were enqueued.
    pub subjects: Vec<String>,
    /// `HEAD` afterwards: the new commit, or the old `HEAD` when there was
    /// nothing to commit, or `None` in a repository with no commit yet.
    pub sha: Option<String>,
    /// How many attempts it took.
    pub attempts: u32,
}

/// A batch that landed, as [`Hooks::on_landing`] hears of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landing {
    /// What its tickets are answered with.
    pub outcome: CommitOutcome,
    /// What git ignored when it landed (see the module contract), or why the
    /// batch's status could not say.
    pub ignored: Result<Ignored, String>,
}

/// What git ignores in the Agora, and why some of it is out of history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ignored {
    /// Every entry git ignores, as the status listed it: a file, or a
    /// directory a pattern matched, ending in `/`.
    pub entries: Vec<String>,
    /// The git repositories nested in the Agora that the committer keeps out
    /// of its history, each a path from the root with no trailing `/`. Each is
    /// among `entries` too, as the directory.
    pub nested: Vec<String>,
}

/// A subject whose batch did not land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFailure {
    /// The subject that was not committed.
    pub subject: String,
    /// Why, in git's words or a fault's.
    pub reason: String,
}

/// What open found at `.git/index.lock`.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexLock {
    /// There was none, or the git holding it finished within the schedule.
    Absent,
    /// One outlived the schedule and was removed.
    Removed,
    /// One outlived the schedule and could not be removed, with the OS's
    /// reason. On Windows a refusal (OS error 5 or 32) means some process
    /// holds it open; a delete that succeeds proves nothing, because git closes
    /// the file while it waits to rename it (measured, 2026-10-20). The
    /// commit's retries come back to it once it is older than [`STALE_AFTER`].
    Stuck(String),
}

/// Why an Agora could not be opened. Each says what to do about it.
#[derive(Debug)]
pub enum OpenError {
    /// Another committer holds this lock file.
    Held(PathBuf),
    /// `.git` here is not a directory.
    NotARepository(PathBuf),
    /// `git init` failed, in git's words.
    Init(String),
    /// The directory or the lock could not be opened.
    Io(io::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Held(lock) => write!(
                f,
                "another committer holds this Agora ({}), and an Agora has exactly one (ADR-0004). Stop the other harness on this home, or set EPH_HOME to another home",
                lock.display()
            ),
            OpenError::NotARepository(dot_git) => write!(
                f,
                "{} is not a directory, so this is not a repository the harness made. Move the Agora aside and start again",
                dot_git.display()
            ),
            OpenError::Init(stderr) => write!(f, "git init failed: {stderr}"),
            OpenError::Io(error) => write!(f, "the Agora could not be opened: {error}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<io::Error> for OpenError {
    fn from(error: io::Error) -> OpenError {
        OpenError::Io(error)
    }
}

/// An opened Agora.
#[must_use]
#[derive(Debug)]
pub struct Opened {
    /// Its committer.
    pub committer: Committer,
    /// Whether this open ran `git init`: there was no repository yet.
    pub initialized: bool,
    /// What open found at `.git/index.lock`, for the boot to report.
    pub index_lock: IndexLock,
    /// The lock files a `git init` killed before it finished had left directly
    /// in `.git`, removed before this open's `git init`, for the boot to report.
    /// Empty unless [`Opened::initialized`].
    pub init_locks: Vec<String>,
}

/// The answer to one [`Committer::commit`]. Dropping it is allowed: a failure
/// nobody waits for is reported and kept.
#[must_use = "a dropped ticket's failure is only reported, never returned"]
#[derive(Debug)]
pub struct Ticket {
    receiver: Receiver<Result<CommitOutcome, CommitFailure>>,
    subject: String,
}

impl Ticket {
    /// Waits until this subject's batch has landed or given up.
    pub fn wait(self) -> Result<CommitOutcome, CommitFailure> {
        let subject = self.subject;
        self.receiver
            .recv()
            .unwrap_or_else(|_| Err(stopped(subject)))
    }
}

/// The Agora's committer. See the module contract.
#[derive(Debug)]
pub struct Committer {
    root: PathBuf,
    requests: Option<Sender<Request>>,
    worker: Option<JoinHandle<()>>,
    record: Arc<Record>,
}

impl Committer {
    /// Opens the Agora at `root`. See the module contract for the order: the
    /// directory, the lock, `git init -b main` (with no template) when there is
    /// no repository yet, the stale `index.lock`, then the queue.
    pub fn open(root: &Path, hooks: Hooks) -> Result<Opened, OpenError> {
        Committer::open_with(root, hooks, Git::agora())
    }

    /// [`Committer::open`] with a runner a test chose.
    pub(crate) fn open_with(root: &Path, hooks: Hooks, git: Git) -> Result<Opened, OpenError> {
        let root = path::absolute(root)?;
        let dot_git = root.join(".git");
        if fs::symlink_metadata(&dot_git).is_ok_and(|meta| !meta.is_dir()) {
            return Err(OpenError::NotARepository(dot_git));
        }
        fs::create_dir_all(&dot_git)?;
        let lock_path = dot_git.join(LOCK_FILE);
        let Some(lock) = acquire(&lock_path)? else {
            return Err(OpenError::Held(lock_path));
        };
        // Only the lock's holder may make the repository, so two opens of a new
        // Agora cannot both run `init`, and a `.git` a crash left half made is
        // finished here.
        let initialized = !dot_git.join("HEAD").exists();
        let mut init_locks = Vec::new();
        if initialized {
            init_locks = settle_init_locks(&dot_git)?;
            let init = git.run(&root, &["init", "--template=", "-b", "main"]);
            if !init.ok() {
                return Err(OpenError::Init(init.stderr.trim().to_string()));
            }
        }
        let record = Arc::new(Record {
            kept: Mutex::new(VecDeque::new()),
            report: hooks.report.clone(),
        });
        let worker = Worker {
            root: root.clone(),
            git,
            hooks,
            record: Arc::clone(&record),
            nested: RefCell::new(Nested::load(&dot_git)),
            _lock: lock,
        };
        let index_lock = worker.settle_index_lock();
        let (sender, receiver) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("eph-git-committer".to_string())
            .spawn(move || worker.serve(&receiver))?;
        Ok(Opened {
            committer: Committer {
                root,
                requests: Some(sender),
                worker: Some(handle),
                record,
            },
            initialized,
            index_lock,
            init_locks,
        })
    }

    /// The Agora's root, absolute.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Queues a commit of everything in the working tree, under `subject`.
    pub fn commit(&self, subject: impl Into<String>) -> Ticket {
        let subject = subject.into();
        let (reply, receiver) = mpsc::channel();
        self.send(Request::Commit(Pending {
            subject: subject.clone(),
            reply: Some(reply),
        }));
        Ticket { receiver, subject }
    }

    /// Queues a commit nobody waits for. Its failure is reported and kept.
    pub fn commit_soon(&self, subject: impl Into<String>) {
        self.send(Request::Commit(Pending {
            subject: subject.into(),
            reply: None,
        }));
    }

    /// Commits whatever a killed harness left, under [`RECONCILE_SUBJECT`].
    /// Its stale `index.lock` was dealt with by open ([`Opened::index_lock`]).
    pub fn reconcile(&self) -> Ticket {
        self.commit(RECONCILE_SUBJECT)
    }

    /// Waits until everything queued before this call has landed or given up.
    pub fn drained(&self) {
        self.ask(Query::Drained, || ());
    }

    /// `HEAD`, or `None` in a repository with no commit yet.
    pub fn head(&self) -> Option<String> {
        self.ask(Query::Head, || None)
    }

    /// Whether the working tree holds anything not yet committed: `false` when
    /// git cannot say, as today.
    pub fn is_dirty(&self) -> bool {
        self.ask(Query::Dirty, || false)
    }

    /// What git ignores in the working tree now, as a landing lists it (see the
    /// module contract), without staging or committing anything: for a boot
    /// that commits nothing.
    pub fn ignored(&self) -> Result<Ignored, String> {
        self.ask(Query::Ignored, || {
            Err("the committer stopped before it answered".to_string())
        })
    }

    /// The failures nobody waited for, oldest first.
    pub fn failures(&self) -> Vec<CommitFailure> {
        guard(&self.record.kept).iter().cloned().collect()
    }

    /// Queues `request`. One the queue cannot take is answered as stopped: a
    /// ticket's sender is dropped with it, and a commit nobody waits for is
    /// kept as a failure rather than lost.
    fn send(&self, request: Request) {
        let refused = match &self.requests {
            Some(requests) => requests.send(request).err().map(|refused| refused.0),
            None => Some(request),
        };
        if let Some(Request::Commit(Pending {
            subject,
            reply: None,
        })) = refused
        {
            self.record.keep(stopped(subject));
        }
    }

    fn ask<T>(&self, query: impl FnOnce(Sender<T>) -> Query, stopped: impl FnOnce() -> T) -> T {
        let (reply, receiver) = mpsc::channel();
        self.send(Request::Query(query(reply)));
        receiver.recv().unwrap_or_else(|_| stopped())
    }
}

impl Drop for Committer {
    fn drop(&mut self) {
        // Closing the queue lets the worker land what is queued and stop; the
        // lock is released only once its last git has finished.
        drop(self.requests.take());
        if let Some(worker) = self.worker.take() {
            let _joined = worker.join();
        }
    }
}

/// One batch's message: the first subject, then every subject in the body.
/// Never empty.
pub fn commit_message(subjects: &[String]) -> String {
    let first = subjects
        .first()
        .map(|subject| subject.trim())
        .filter(|subject| !subject.is_empty())
        .unwrap_or(BLANK_SUBJECT);
    if subjects.len() < 2 {
        return first.to_string();
    }
    let listed: Vec<String> = subjects
        .iter()
        .map(|subject| format!("- {subject}"))
        .collect();
    format!(
        "{first} (+{} more)\n\n{}",
        subjects.len() - 1,
        listed.join("\n")
    )
}

/// The lock files directly in `dot_git` that a killed `git init` can leave,
/// such as `config.lock`: every `*.lock` there except the committer's own,
/// which open holds, and `index.lock`, which open settles on its own.
fn init_locks(dot_git: &Path) -> io::Result<Vec<PathBuf>> {
    let mut locks = Vec::new();
    for entry in fs::read_dir(dot_git)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".lock") && name != LOCK_FILE && name != "index.lock" {
            locks.push(entry.path());
        }
    }
    locks.sort();
    Ok(locks)
}

/// Waits while [`init_locks`] finds any, over the retry schedule, so a git
/// still running for a killed harness can finish; then removes those left, and
/// names the ones it removed. One that cannot be removed is left for `git init`
/// to refuse in its own words.
fn settle_init_locks(dot_git: &Path) -> io::Result<Vec<String>> {
    let mut waits = RETRY_WAITS.iter();
    loop {
        let locks = init_locks(dot_git)?;
        if locks.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(wait) = waits.next() {
            thread::sleep(Duration::from_millis(*wait));
            continue;
        }
        return Ok(locks
            .iter()
            .filter(|lock| fs::remove_file(lock).is_ok())
            .filter_map(|lock| lock.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect());
    }
}

/// Takes the committer's lock, asking again over [`LOCK_WAITS`] while it reads
/// as held.
fn acquire(path: &Path) -> io::Result<Option<Exclusive>> {
    for wait in LOCK_WAITS {
        if let Some(lock) = Exclusive::try_acquire(path)? {
            return Ok(Some(lock));
        }
        thread::sleep(Duration::from_millis(wait));
    }
    Exclusive::try_acquire(path)
}

fn stopped(subject: String) -> CommitFailure {
    CommitFailure {
        subject,
        reason: "the committer stopped before it answered".to_string(),
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The failures nobody waited for, and who hears of them.
struct Record {
    kept: Mutex<VecDeque<CommitFailure>>,
    report: Vec<Arc<ReportHook>>,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("kept", &guard(&self.kept).len())
            .field("reporters", &self.report.len())
            .finish()
    }
}

impl Record {
    /// Reports `failure` to every reporter, then keeps it, dropping the oldest
    /// past [`MAX_RECORDED_FAILURES`]. The reporters run before the lock is
    /// taken, so they may read [`Committer::failures`].
    fn keep(&self, failure: CommitFailure) {
        for report in &self.report {
            report(&failure);
        }
        let mut kept = guard(&self.kept);
        kept.push_back(failure);
        // One is added at a time, so one is dropped at most: an `if`, which no
        // change to its condition can turn into a loop (mutation round 2).
        if kept.len() > MAX_RECORDED_FAILURES {
            kept.pop_front();
        }
    }
}

enum Request {
    Commit(Pending),
    Query(Query),
}

struct Pending {
    subject: String,
    reply: Option<Sender<Result<CommitOutcome, CommitFailure>>>,
}

enum Query {
    Drained(Sender<()>),
    Head(Sender<Option<String>>),
    Dirty(Sender<bool>),
    Ignored(Sender<Result<Ignored, String>>),
}

/// How one attempt ended. A landing carries what the batch's status ignored.
enum Attempt {
    Landed(Result<Ignored, String>),
    Failed(String),
    Aborted(String),
}

/// The status a batch reads after staging, and [`Committer::ignored`] reads:
/// porcelain v2, so a gitlink says it is one, NUL-separated, so no path is
/// quoted, and the ignored entries listed as the patterns matched them. The
/// untracked mode is named so a repository setting
/// (`status.showUntrackedFiles=no`) cannot hide them.
const STATUS: [&str; 5] = [
    "status",
    "--porcelain=v2",
    "-z",
    "--ignored=matching",
    "--untracked-files=normal",
];

/// The status that finds a nested repository no commit is in: with every
/// untracked file listed one by one, an untracked entry that is a directory is
/// a repository git would not descend into (measured, git 2.53).
const UNTRACKED: [&str; 4] = ["status", "--porcelain=v2", "-z", "--untracked-files=all"];

/// Where the committer lists the nested repositories it keeps out: git's own
/// per-repository exclude file, between [`NESTED_BEGIN`] and [`NESTED_END`].
/// Any other line in it is kept as it was.
pub const EXCLUDE_FILE: &str = "info/exclude";

/// The first line of the committer's block in [`EXCLUDE_FILE`].
pub const NESTED_BEGIN: &str =
    "# >>> ephesus: git repositories nested in the Agora, kept out of its history by the committer";

/// The last line of the committer's block in [`EXCLUDE_FILE`].
pub const NESTED_END: &str = "# <<< ephesus";

/// What one status said.
#[derive(Debug, PartialEq, Eq)]
struct StatusRead {
    /// Whether it listed anything other than ignored entries: something staged
    /// that a commit would take, or a change made since the staging.
    changed: bool,
    /// Gitlinks it listed: nested repositories `add -A` staged.
    gitlinks: Vec<String>,
    /// The ignored entries and the nested repositories kept out, or why the
    /// status could not say.
    ignored: Result<Ignored, String>,
}

/// What one porcelain v2 status lists.
#[derive(Debug, Default, PartialEq, Eq)]
struct StatusFields {
    changed: bool,
    gitlinks: Vec<String>,
    ignored: Vec<String>,
    untracked: Vec<String>,
}

/// Reads porcelain v2 status with `-z`. A changed entry is `1 XY sub mH mI mW
/// hH hI path`, a rename or copy `2 … X<score> path` followed by its source
/// path as one more field, an unmerged one `u …` with ten fields before its
/// path, an untracked one `? path` and an ignored one `! path`. `sub` begins
/// with `S` for a gitlink. The Agora's mail moves between state directories,
/// which a status after `add -A` shows as renames.
fn read_status(stdout: &str) -> StatusFields {
    let mut read = StatusFields::default();
    let mut fields = stdout.split('\0').filter(|field| !field.is_empty());
    while let Some(field) = fields.next() {
        let path = field.get(2..).unwrap_or_default();
        let (before_path, gitlink) = match field.as_bytes().first() {
            Some(b'!') => {
                read.ignored.push(path.to_string());
                continue;
            }
            Some(b'?') => {
                read.untracked.push(path.to_string());
                read.changed = true;
                continue;
            }
            Some(b'#') => continue,
            Some(b'1') => (8, is_gitlink(field)),
            Some(b'2') => {
                let _source = fields.next();
                (9, is_gitlink(field))
            }
            _ => (10, false),
        };
        read.changed = true;
        if gitlink && let Some(path) = field.splitn(before_path + 1, ' ').nth(before_path) {
            read.gitlinks.push(path.to_string());
        }
    }
    read
}

/// Whether a changed entry's `sub` field marks a gitlink.
fn is_gitlink(field: &str) -> bool {
    field
        .split(' ')
        .nth(2)
        .is_some_and(|sub| sub.starts_with('S'))
}

/// The committer's block of [`EXCLUDE_FILE`] for `nested`, after `kept`, the
/// lines of the file that are not its own.
fn exclude_text(kept: &[String], nested: &[String]) -> String {
    let mut text = String::new();
    for line in kept {
        text.push_str(line);
        text.push('\n');
    }
    if !nested.is_empty() {
        text.push_str(NESTED_BEGIN);
        text.push('\n');
        for path in nested {
            text.push('/');
            text.push_str(&escape_pattern(path));
            text.push_str("/\n");
        }
        text.push_str(NESTED_END);
        text.push('\n');
    }
    text
}

/// Splits [`EXCLUDE_FILE`]'s text into the lines that are not the
/// committer's, and the nested repositories its block lists.
fn read_exclude(text: &str) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut nested = Vec::new();
    let mut ours = false;
    for line in text.lines() {
        match line {
            NESTED_BEGIN => ours = true,
            NESTED_END => ours = false,
            _ if ours => {
                if let Some(path) = line
                    .strip_prefix('/')
                    .and_then(|rest| rest.strip_suffix('/'))
                {
                    nested.push(unescape_pattern(path));
                }
            }
            _ => kept.push(line.to_string()),
        }
    }
    (kept, nested)
}

/// `path` as an ignore pattern that matches only itself: git's wildcards and
/// its escape character escaped. The pattern is anchored by the leading `/`
/// the caller adds, so no other character is special.
fn escape_pattern(path: &str) -> String {
    let mut pattern = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern
}

/// The path [`escape_pattern`] escaped.
fn unescape_pattern(pattern: &str) -> String {
    let mut path = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(escaped) = chars.next()
        {
            path.push(escaped);
            continue;
        }
        path.push(c);
    }
    path
}

/// The thread that owns the Agora's git and the committer's lock.
struct Worker {
    root: PathBuf,
    git: Git,
    hooks: Hooks,
    record: Arc<Record>,
    /// The nested repositories kept out, and the exclude file's other lines.
    nested: RefCell<Nested>,
    _lock: Exclusive,
}

/// What the committer keeps in [`EXCLUDE_FILE`].
#[derive(Debug, Default)]
struct Nested {
    /// The file's lines that are not the committer's, kept as they were.
    kept: Vec<String>,
    /// The nested repositories it keeps out.
    paths: Vec<String>,
}

impl Nested {
    /// What `dot_git`'s exclude file holds now; an absent or unreadable one
    /// holds nothing.
    fn load(dot_git: &Path) -> Nested {
        let text = fs::read_to_string(dot_git.join(EXCLUDE_FILE)).unwrap_or_default();
        let (kept, paths) = read_exclude(&text);
        Nested { kept, paths }
    }
}

impl Worker {
    fn serve(self, requests: &Receiver<Request>) {
        while let Ok(first) = requests.recv() {
            let mut batch = Vec::new();
            for request in iter::once(first).chain(requests.try_iter()) {
                match request {
                    Request::Commit(pending) => batch.push(pending),
                    Request::Query(query) => {
                        self.land(std::mem::take(&mut batch));
                        self.answer(query);
                    }
                }
            }
            self.land(batch);
        }
    }

    fn answer(&self, query: Query) {
        match query {
            Query::Drained(reply) => {
                let _sent = reply.send(());
            }
            Query::Head(reply) => {
                let _sent = reply.send(self.head());
            }
            Query::Dirty(reply) => {
                let _sent = reply.send(self.is_dirty());
            }
            Query::Ignored(reply) => {
                let _sent = reply.send(self.status().ignored);
            }
        }
    }

    fn land(&self, batch: Vec<Pending>) {
        if batch.is_empty() {
            return;
        }
        let subjects: Vec<String> = batch
            .iter()
            .map(|pending| pending.subject.clone())
            .collect();
        let landed = self.run_commit(subjects).map(|(outcome, ignored)| {
            let landing = Landing { outcome, ignored };
            for hook in &self.hooks.landing {
                hook(&landing);
            }
            landing.outcome
        });
        for pending in batch {
            let result = landed.clone().map_err(|reason| CommitFailure {
                subject: pending.subject,
                reason,
            });
            let unheard = match pending.reply {
                Some(reply) => reply.send(result).err().map(|unsent| unsent.0),
                None => Some(result),
            };
            // A failure nobody is waiting for, whether no ticket was asked for
            // or it was dropped, is reported and kept.
            if let Some(Err(failure)) = unheard {
                self.record.keep(failure);
            }
        }
    }

    fn run_commit(
        &self,
        subjects: Vec<String>,
    ) -> Result<(CommitOutcome, Result<Ignored, String>), String> {
        let message = commit_message(&subjects);
        let mut last = String::new();
        let waits = iter::once(None).chain(RETRY_WAITS.map(Some));
        for (attempt, wait) in (1..).zip(waits) {
            if let Some(wait) = wait {
                thread::sleep(Duration::from_millis(wait));
                self.clear_stale_index_lock();
            }
            match self.attempt(&message) {
                Attempt::Landed(ignored) => {
                    let outcome = CommitOutcome {
                        subjects,
                        sha: self.head(),
                        attempts: attempt,
                    };
                    return Ok((outcome, ignored));
                }
                Attempt::Failed(reason) => last = reason,
                Attempt::Aborted(reason) => return Err(reason),
            }
        }
        Err(format!(
            "the Agora's commit gave up after {} attempts (subjects: {}): {last}",
            RETRY_WAITS.len() + 1,
            subjects.join("; ")
        ))
    }

    fn attempt(&self, message: &str) -> Attempt {
        if let Err(reason) = self.hooks.check(FaultPoint::BeforeStage) {
            return Attempt::Aborted(reason);
        }
        if let Err(reason) = self.prune_nested() {
            return Attempt::Failed(reason);
        }
        let staged = self.git(&["add", "-A"]);
        if !staged.ok() {
            // A repository an agent made with no commit in it fails every
            // `add -A`, every agent's work included: keep it out, then retry.
            let reason = staged.stderr.trim().to_string();
            return match self.keep_out_untracked_repositories() {
                Ok(true) => Attempt::Failed(format!(
                    "{reason}; the nested repositories this found are now kept out of the Agora, and the next attempt stages the rest"
                )),
                Ok(false) => Attempt::Failed(reason),
                Err(why) => Attempt::Failed(format!("{reason}; and {why}")),
            };
        }
        if let Err(reason) = self.hooks.check(FaultPoint::AfterStage) {
            return Attempt::Aborted(reason);
        }
        // Nothing staged is a landing, whatever git ignores; a status that
        // fails is not, and the commit after it decides.
        let mut status = self.status();
        if !status.gitlinks.is_empty() {
            // A repository an agent made with a commit in it was staged as a
            // gitlink, which keeps its files out of history: keep it out and
            // unstage it, so its pointer never lands either.
            if let Err(reason) = self.unstage_gitlinks(std::mem::take(&mut status.gitlinks)) {
                return Attempt::Failed(reason);
            }
            status = self.status();
        }
        if !status.changed {
            return Attempt::Landed(status.ignored);
        }
        if let Err(reason) = self.hooks.check(FaultPoint::BeforeCommit) {
            return Attempt::Aborted(reason);
        }
        let file = self.root.join(".git").join(MESSAGE_FILE);
        let relative = format!(".git/{MESSAGE_FILE}");
        if let Err(error) = write_atomic(&file, message.as_bytes()) {
            return Attempt::Failed(format!("the commit message could not be written: {error}"));
        }
        let committed = self.git(&["commit", "-F", &relative]);
        if !committed.ok() {
            return Attempt::Failed(committed.stderr.trim().to_string());
        }
        if let Err(reason) = self.hooks.check(FaultPoint::AfterCommit) {
            return Attempt::Aborted(reason);
        }
        // What was ignored before the commit still is: a commit takes only
        // what was staged.
        Attempt::Landed(status.ignored)
    }

    /// The working tree's [`STATUS`]. A status that fails changes nothing, so
    /// its `changed` is `true`: the commit after it decides.
    fn status(&self) -> StatusRead {
        let status = self.git(&STATUS);
        if !status.ok() {
            return StatusRead {
                changed: true,
                gitlinks: Vec::new(),
                ignored: Err(format!(
                    "git status failed, so what git ignores is unknown: {}",
                    status.stderr.trim()
                )),
            };
        }
        let read = read_status(&status.stdout);
        StatusRead {
            changed: read.changed,
            gitlinks: read.gitlinks,
            ignored: Ok(Ignored {
                entries: read.ignored,
                nested: self.nested.borrow().paths.clone(),
            }),
        }
    }

    /// Drops from the kept-out list every repository whose `.git` is gone, so
    /// a directory that stopped being a repository is committed again.
    fn prune_nested(&self) -> Result<(), String> {
        let gone = self
            .nested
            .borrow()
            .paths
            .iter()
            .any(|path| fs::symlink_metadata(self.root.join(path).join(".git")).is_err());
        if !gone {
            return Ok(());
        }
        let root = self.root.clone();
        self.change_nested(|paths| {
            paths.retain(|path| fs::symlink_metadata(root.join(path).join(".git")).is_ok());
        })
    }

    /// Finds the nested repositories no commit is in, keeps the new ones out,
    /// and says whether there were any.
    fn keep_out_untracked_repositories(&self) -> Result<bool, String> {
        let listed = self.git(&UNTRACKED);
        if !listed.ok() {
            return Err(format!(
                "the nested repositories could not be listed: {}",
                listed.stderr.trim()
            ));
        }
        let found: Vec<String> = read_status(&listed.stdout)
            .untracked
            .into_iter()
            .filter_map(|entry| entry.strip_suffix('/').map(str::to_string))
            .collect();
        self.keep_out(found)
    }

    /// Keeps `gitlinks` out and takes them out of the index, leaving every file
    /// on disk.
    fn unstage_gitlinks(&self, gitlinks: Vec<String>) -> Result<(), String> {
        let mut args = vec!["rm", "--cached", "-f", "-q", "--ignore-unmatch", "--"];
        args.extend(gitlinks.iter().map(String::as_str));
        self.keep_out(gitlinks.clone())?;
        let unstaged = self.git(&args);
        if !unstaged.ok() {
            return Err(format!(
                "the nested repositories {} could not be unstaged: {}",
                gitlinks.join(", "),
                unstaged.stderr.trim()
            ));
        }
        Ok(())
    }

    /// Adds the paths of `found` the list does not hold, and says whether it
    /// added any. A path a line cannot hold (a line break) is left out.
    fn keep_out(&self, found: Vec<String>) -> Result<bool, String> {
        let new: Vec<String> = found
            .into_iter()
            .filter(|path| !path.contains(['\n', '\r']))
            .filter(|path| !self.nested.borrow().paths.contains(path))
            .collect();
        if new.is_empty() {
            return Ok(false);
        }
        self.change_nested(|paths| paths.extend(new))?;
        Ok(true)
    }

    /// Changes the kept-out list with `change` and writes the exclude file. The
    /// list in memory changes only once the file holds it, so a failed write
    /// is tried again by the next attempt rather than believed.
    fn change_nested(&self, change: impl FnOnce(&mut Vec<String>)) -> Result<(), String> {
        let mut nested = self.nested.borrow_mut();
        let mut paths = nested.paths.clone();
        change(&mut paths);
        let dot_git = self.root.join(".git");
        let text = exclude_text(&nested.kept, &paths);
        fs::create_dir_all(dot_git.join("info"))
            .and_then(|()| write_atomic(&dot_git.join(EXCLUDE_FILE), text.as_bytes()))
            .map_err(|error| {
                format!("the Agora's list of nested repositories could not be written: {error}")
            })?;
        nested.paths = paths;
        Ok(())
    }

    /// Waits while `index.lock` exists, over the retry schedule, so a git left
    /// running by a killed harness can finish; then removes one that outlived it.
    fn settle_index_lock(&self) -> IndexLock {
        let lock = self.index_lock();
        let mut waits = RETRY_WAITS.iter();
        while lock.exists() {
            match waits.next() {
                Some(wait) => thread::sleep(Duration::from_millis(*wait)),
                // A lock that vanished in the instant since the last look reads
                // as stuck with the OS's "not found": a race no test can stage,
                // and the commit's retries then find nothing (mutation round 2).
                None => {
                    return match fs::remove_file(&lock) {
                        Ok(()) => IndexLock::Removed,
                        Err(error) => IndexLock::Stuck(error.to_string()),
                    };
                }
            }
        }
        IndexLock::Absent
    }

    /// Removes an `index.lock` older than [`STALE_AFTER`], and nothing younger.
    /// One that cannot be removed fails the next attempt in git's own words,
    /// which the give-up reason carries.
    fn clear_stale_index_lock(&self) {
        let lock = self.index_lock();
        let age = fs::metadata(&lock)
            .and_then(|meta| meta.modified())
            .map(|modified| modified.elapsed().unwrap_or_default());
        if age.is_ok_and(|age| age >= STALE_AFTER) {
            let _removed = fs::remove_file(&lock);
        }
    }

    fn index_lock(&self) -> PathBuf {
        self.root.join(".git").join("index.lock")
    }

    fn head(&self) -> Option<String> {
        let head = self.git(&["rev-parse", "HEAD"]);
        head.ok().then(|| head.stdout.trim().to_string())
    }

    /// Whether `status` lists anything; `false` when `status` itself fails.
    fn is_dirty(&self) -> bool {
        let status = self.git(&["status", "--porcelain"]);
        status.ok() && !status.stdout.trim().is_empty()
    }

    fn git(&self, args: &[&str]) -> GitResult {
        self.git.run(&self.root, args)
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::runner::tests::{host_with, link_dir, must, repo_with_commit, scratch, write};

    /// The variable naming the Agora a child process works on.
    const CHILD_AGORA: &str = "EPH_GIT_TEST_AGORA";
    /// The variable naming what a child process does.
    const CHILD_MODE: &str = "EPH_GIT_TEST_MODE";
    /// The exit code of a child that died at its fault point.
    const DIED: i32 = 86;
    /// The longest any test waits on a child: well inside a mutation round's
    /// per-mutant timeout, so a mutant that hangs a child fails, not times out.
    const CHILD_PATIENCE: Duration = Duration::from_secs(10);

    fn open(root: &Path) -> Committer {
        Committer::open(root, Hooks::none()).unwrap().committer
    }

    /// Hooks whose faults refuse once, at `at`, with `reason`.
    fn once_at(at: FaultPoint, reason: &'static str) -> Hooks {
        let armed = AtomicBool::new(true);
        Hooks::none().with_faults(move |point| {
            if point == at && armed.swap(false, Ordering::SeqCst) {
                return Err(reason.to_string());
            }
            Ok(())
        })
    }

    /// Hooks whose faults refuse at `at` while `armed` holds.
    fn armed_at(at: FaultPoint, reason: &'static str, armed: &Arc<AtomicBool>) -> Hooks {
        let armed = Arc::clone(armed);
        Hooks::none().with_faults(move |point| {
            if point == at && armed.load(Ordering::SeqCst) {
                return Err(reason.to_string());
            }
            Ok(())
        })
    }

    /// A fresh Agora with one commit, its committer dropped.
    fn agora(name: &str) -> (PathBuf, PathBuf) {
        let dir = scratch(name);
        let root = dir.join("agora");
        let committer = open(&root);
        write(&root.join("PROTOCOL.md"), "# protocol\n");
        committer.commit("seed").wait().unwrap();
        drop(committer);
        (dir, root)
    }

    fn agora_git(root: &Path, args: &[&str]) -> String {
        let result = Git::agora().run(root, args);
        assert!(result.ok(), "git {args:?}: {result:?}");
        result.stdout
    }

    fn commits(root: &Path) -> usize {
        agora_git(root, &["rev-list", "--count", "HEAD"])
            .trim()
            .parse()
            .unwrap()
    }

    fn tracked(root: &Path, path: &str) -> bool {
        Git::agora()
            .run(root, &["cat-file", "-e", &format!("HEAD:{path}")])
            .ok()
    }

    /// Sets `path`'s modification time to the year 2000, through git's shell.
    fn age(root: &Path, path: &str) {
        agora_git(
            root,
            &[
                "-c",
                &format!("alias.eph-age=!touch -t 200001010000 {path}"),
                "eph-age",
            ],
        );
    }

    #[test]
    fn a_new_agora_is_made_on_main_and_committed_as_ephesus() {
        let dir = scratch("new-agora");
        let root = dir.join("agora");
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert!(opened.initialized);
        assert_eq!(opened.index_lock, IndexLock::Absent);
        let committer = opened.committer;
        assert_eq!(committer.root(), root);
        assert_eq!(committer.head(), None, "no commit yet");
        assert!(
            !root.join(".git").join("hooks").exists() && !root.join(".git").join("info").exists(),
            "no template, not even the system's"
        );
        write(&root.join("registry.json"), "{}\n");
        assert!(committer.is_dirty());
        let outcome = committer.commit("seed the Agora").wait().unwrap();
        assert_eq!(outcome.subjects, ["seed the Agora"]);
        assert_eq!(outcome.attempts, 1);
        let sha = outcome.sha.unwrap();
        assert_eq!(sha.len(), 40, "{sha}");
        assert_eq!(committer.head().as_deref(), Some(sha.as_str()));
        assert!(!committer.is_dirty());
        assert_eq!(
            agora_git(&root, &["log", "-1", "--format=%an <%ae>|%cn <%ce>|%s"]).trim(),
            "Ephesus <harness@ephesus.local>|Ephesus <harness@ephesus.local>|seed the Agora"
        );
        assert_eq!(
            agora_git(&root, &["branch", "--show-current"]).trim(),
            "main"
        );
        assert_eq!(
            agora_git(&root, &["rev-parse", "--show-object-format"]).trim(),
            "sha1",
            "the empty tree the Agora reads attributes from is SHA-1's (runner::EMPTY_TREE)"
        );
        drop(committer);
        let again = Committer::open(&root, Hooks::none()).unwrap();
        assert!(!again.initialized, "a second open makes nothing");
        assert_eq!(commits(&root), 1);
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dot_git_a_crash_left_half_made_is_finished() {
        let dir = scratch("half-made");
        let root = dir.join("agora");
        write(&root.join(".git").join(LOCK_FILE), "");
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert!(
            opened.initialized,
            "a .git with no HEAD is no repository yet"
        );
        write(&root.join("m.json"), "{}\n");
        opened.committer.commit("first").wait().unwrap();
        assert!(tracked(&root, "m.json"));
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_committer_is_refused_until_the_first_is_gone() {
        let (dir, root) = agora("second");
        let first = open(&root);
        let refused = Committer::open(&root, Hooks::none()).unwrap_err();
        let lock = root.join(".git").join(LOCK_FILE);
        assert!(
            matches!(&refused, OpenError::Held(path) if *path == lock),
            "{refused:?}"
        );
        let said = refused.to_string();
        assert!(
            said.contains("another committer holds this Agora"),
            "{said}"
        );
        assert!(said.contains("set EPH_HOME to another home"), "{said}");
        drop(first);
        drop(open(&root));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_released_a_moment_ago_is_taken_not_refused() {
        let (dir, root) = agora("released");
        let lock_path = root.join(".git").join(LOCK_FILE);
        // A fork's copy of the lock's file stands in, held for 60 ms. Taking it
        // waits too: the helper's committer released it a moment ago, and a git
        // another test forked can still hold a copy (R1.4a, Linux, 2026-10-25).
        let held = acquire(&lock_path).unwrap().unwrap();
        let opened = thread::scope(|scope| {
            scope.spawn(move || {
                thread::sleep(Duration::from_millis(60));
                drop(held);
            });
            Committer::open(&root, Hooks::none())
        });
        assert!(opened.is_ok(), "{opened:?}");
        drop(opened);
        let first = open(&root);
        let started = Instant::now();
        assert!(matches!(
            Committer::open(&root, Hooks::none()),
            Err(OpenError::Held(_))
        ));
        let waited: u64 = LOCK_WAITS.iter().sum();
        assert!(
            started.elapsed() >= Duration::from_millis(waited),
            "a held lock is refused only after the waits"
        );
        drop(first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_committer_through_a_link_is_refused_too() {
        let (dir, root) = agora("second-link");
        let first = open(&root);
        let link = dir.join("via-link");
        link_dir(&root, &link);
        assert!(matches!(
            Committer::open(&link, Hooks::none()),
            Err(OpenError::Held(_))
        ));
        drop(first);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dot_git_that_is_not_a_directory_is_refused() {
        let dir = scratch("dot-git-file");
        let root = dir.join("agora");
        write(&root.join(".git"), "gitdir: elsewhere\n");
        let refused = Committer::open(&root, Hooks::none()).unwrap_err();
        assert!(matches!(&refused, OpenError::NotARepository(path) if *path == root.join(".git")));
        assert!(
            refused.to_string().contains("is not a directory"),
            "{refused}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_agora_that_cannot_be_made_says_why() {
        let dir = scratch("cannot-make");
        let file = dir.join("a-file");
        write(&file, "x");
        let io = Committer::open(&file.join("agora"), Hooks::none()).unwrap_err();
        assert!(matches!(io, OpenError::Io(_)), "{io:?}");
        assert!(
            io.to_string()
                .starts_with("the Agora could not be opened: "),
            "{io}"
        );
        // A deadline already past stops `git init` before it can finish, unless
        // git exits before the runner first looks, which a fast `git init` on
        // Linux sometimes does; a git that finished is rightly not a failure. So
        // fresh directories are tried until the deadline wins (R1.4a, 2026-10-25).
        let (root, init) = (0..20)
            .find_map(|attempt| {
                let root = dir.join(format!("agora-{attempt}"));
                match Committer::open_with(
                    &root,
                    Hooks::none(),
                    Git::agora().with_timeout(Duration::ZERO),
                ) {
                    Ok(opened) => {
                        drop(opened);
                        None
                    }
                    Err(error) => Some((root, error)),
                }
            })
            .expect("git init finished inside a zero deadline 20 times running");
        assert!(matches!(init, OpenError::Init(_)), "{init:?}");
        let said = init.to_string();
        assert!(said.starts_with("git init failed: "), "{said}");
        assert!(said.contains("deadline"), "{said}");
        // The lock came first, and the next open leaves a repository a commit
        // lands in. Whether it ran `git init` depends on how far the stopped one
        // got, which the planted tests below fix (R1.4a, 2026-10-25).
        assert!(root.join(".git").join(LOCK_FILE).exists());
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        write(&root.join("m.json"), "{}\n");
        opened.committer.commit("first").wait().unwrap();
        assert!(tracked(&root, "m.json"));
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_git_init_a_kill_left_holding_its_locks_is_finished_and_named() {
        let dir = scratch("init-locks");
        let root = dir.join("agora");
        let dot_git = root.join(".git");
        write(&dot_git.join("config.lock"), "[core]\n");
        write(&dot_git.join("HEAD.lock"), "ref: refs/heads/main\n");
        write(&dot_git.join("index.lock"), "");
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert!(
            opened.initialized,
            "a .git with no HEAD is no repository yet"
        );
        assert_eq!(opened.init_locks, ["HEAD.lock", "config.lock"]);
        assert_eq!(
            opened.index_lock,
            IndexLock::Removed,
            "index.lock is settled and reported on its own"
        );
        assert!(!dot_git.join("config.lock").exists());
        assert!(
            dot_git.join(LOCK_FILE).exists(),
            "the committer's own lock stays"
        );
        assert!(matches!(
            Committer::open(&root, Hooks::none()),
            Err(OpenError::Held(_))
        ));
        write(&root.join("m.json"), "{}\n");
        opened.committer.commit("first").wait().unwrap();
        assert!(tracked(&root, "m.json"));
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_repository_with_a_head_keeps_its_lock_files() {
        let (dir, root) = agora("init-locks-kept");
        let stray = root.join(".git").join("config.lock");
        write(&stray, "[core]\n");
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert!(!opened.initialized);
        assert!(opened.init_locks.is_empty());
        assert!(
            stray.exists(),
            "only a .git with no HEAD is a killed init's"
        );
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_lets_a_live_init_finish_with_its_lock() {
        let dir = scratch("init-live");
        let root = dir.join("agora");
        let lock = root.join(".git").join("config.lock");
        write(&lock, "[core]\n");
        // The stand-in git finishes 30 ms in, far inside open's 375 ms wait.
        let opened = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(30));
                fs::remove_file(&lock).unwrap();
            });
            Committer::open(&root, Hooks::none()).unwrap()
        });
        assert!(opened.initialized);
        assert!(opened.init_locks.is_empty(), "the git finished on its own");
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_host_pointing_git_elsewhere_cannot_move_the_agora() {
        let (dir, root) = agora("host");
        let other = dir.join("other");
        repo_with_commit(&other);
        let other_git = other.join(".git");
        let host = host_with(&[
            ("GIT_DIR", other_git.to_str().unwrap()),
            ("GIT_WORK_TREE", other.to_str().unwrap()),
            ("GIT_AUTHOR_NAME", "Mallory"),
        ]);
        let committer =
            Committer::open_with(&root, Hooks::none(), Git::agora().with_environment(host))
                .unwrap()
                .committer;
        write(&root.join("inbox").join("m1.json"), "{}\n");
        committer.commit("deliver m1").wait().unwrap();
        drop(committer);
        assert!(tracked(&root, "inbox/m1.json"));
        assert_eq!(commits(&root), 2);
        assert_eq!(must(&other, &["rev-list", "--count", "HEAD"]).trim(), "1");
        assert_eq!(
            agora_git(&root, &["log", "-1", "--format=%an"]).trim(),
            "Ephesus"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nothing_to_commit_is_a_landing_that_names_head() {
        let (dir, root) = agora("no-op");
        let committer = open(&root);
        let head = committer.head();
        let outcome = committer.commit("nothing").wait().unwrap();
        assert_eq!(outcome.sha, head);
        assert_eq!(outcome.attempts, 1);
        assert_eq!(commits(&root), 1);
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn work_enqueued_while_a_commit_runs_lands_as_one_commit() {
        let (dir, root) = agora("batch");
        let (entered, parked) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let entered = Mutex::new(entered);
        let released = Mutex::new(released);
        let first = AtomicBool::new(true);
        let hooks = Hooks::none().with_faults(move |point| {
            if point == FaultPoint::AfterCommit && first.swap(false, Ordering::SeqCst) {
                guard(&entered).send(()).unwrap();
                let _resumed = guard(&released).recv_timeout(CHILD_PATIENCE);
            }
            Ok(())
        });
        let committer = Committer::open(&root, hooks).unwrap().committer;
        write(&root.join("a.txt"), "a");
        let a = committer.commit("first");
        parked
            .recv_timeout(CHILD_PATIENCE)
            .expect("the first batch parks at its fault point");
        write(&root.join("b.txt"), "b");
        let b = committer.commit("second");
        let c = committer.commit("third");
        release.send(()).unwrap();
        let a = a.wait().unwrap();
        let b = b.wait().unwrap();
        let c = c.wait().unwrap();
        assert_eq!(a.subjects, ["first"]);
        assert_eq!(b.subjects, ["second", "third"]);
        assert_eq!(b, c, "both subjects share one outcome");
        assert_ne!(a.sha, b.sha);
        assert_eq!(
            agora_git(&root, &["log", "-1", "--format=%B"]).trim_end(),
            "second (+1 more)\n\n- second\n- third"
        );
        assert_eq!(commits(&root), 3);
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_batch_too_long_for_a_command_line_still_lands() {
        let (dir, root) = agora("long-batch");
        let (entered, parked) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let entered = Mutex::new(entered);
        let released = Mutex::new(released);
        let first = AtomicBool::new(true);
        let hooks = Hooks::none().with_faults(move |point| {
            if point == FaultPoint::AfterCommit && first.swap(false, Ordering::SeqCst) {
                guard(&entered).send(()).unwrap();
                let _resumed = guard(&released).recv_timeout(CHILD_PATIENCE);
            }
            Ok(())
        });
        let committer = Committer::open(&root, hooks).unwrap().committer;
        write(&root.join("a.txt"), "a");
        let parked_first = committer.commit("first");
        parked.recv_timeout(CHILD_PATIENCE).unwrap();
        write(&root.join("b.txt"), "b");
        // 600 subjects of 70 characters: past Windows' 32,767-character limit.
        let tickets: Vec<Ticket> = (0..600)
            .map(|n| {
                committer.commit(format!(
                    "hermes: deliver message {n:04} to agent.mason {}",
                    "x".repeat(30)
                ))
            })
            .collect();
        release.send(()).unwrap();
        parked_first.wait().unwrap();
        for ticket in tickets {
            assert_eq!(ticket.wait().unwrap().attempts, 1);
        }
        let body = agora_git(&root, &["log", "-1", "--format=%B"]);
        assert!(body.len() > 40_000, "{}", body.len());
        assert!(body.contains("(+599 more)"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn many_callers_at_once_never_contend_for_the_index() {
        let (dir, root) = agora("many");
        let committer = open(&root);
        // Every file is written while commits run, so an `add -A` can meet one
        // mid-rename and fail once (seen on Windows CI, 2026-10-20): that is
        // what the retries absorb. What one queue rules out is two of its own
        // gits at once, so every caller's work lands.
        thread::scope(|scope| {
            for n in 0..8 {
                let committer = &committer;
                let root = &root;
                scope.spawn(move || {
                    write(&root.join("work").join(format!("{n}.txt")), "w");
                    committer.commit(format!("work {n}")).wait().unwrap();
                });
            }
        });
        assert!(!committer.is_dirty());
        for n in 0..8 {
            assert!(tracked(&root, &format!("work/{n}.txt")));
        }
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_older_than_the_deadline_is_cleared_by_the_retry() {
        let (dir, root) = agora("stale-retry");
        let committer = open(&root);
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        age(&root, ".git/index.lock");
        write(&root.join("m.json"), "{}\n");
        let outcome = committer.commit("after a killed git").wait().unwrap();
        assert_eq!(outcome.attempts, 2, "the first attempt met the lock");
        assert!(!lock.exists());
        assert!(tracked(&root, "m.json"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_young_lock_is_someone_elses_and_is_never_removed() {
        let (dir, root) = agora("young-lock");
        let committer = open(&root);
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        write(&root.join("m.json"), "{}\n");
        let failure = committer
            .commit("while another git works")
            .wait()
            .unwrap_err();
        assert!(
            failure.reason.contains("after 5 attempts"),
            "{}",
            failure.reason
        );
        assert!(failure.reason.contains("index.lock"), "{}", failure.reason);
        assert!(
            lock.exists(),
            "a lock this committer did not leave is left alone"
        );
        fs::remove_file(&lock).unwrap();
        assert_eq!(committer.commit("after it").wait().unwrap().attempts, 1);
        assert!(tracked(&root, "m.json"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_commit_that_keeps_failing_gives_up_naming_its_subjects_and_the_queue_goes_on() {
        let (dir, root) = agora("gives-up");
        let committer = open(&root);
        // A directory where git makes its lock: no attempt can stage, and no
        // retry can clear it, however old it is.
        let lock = root.join(".git").join("index.lock");
        fs::create_dir_all(&lock).unwrap();
        age(&root, ".git/index.lock");
        write(&root.join("m.json"), "{}\n");
        let started = Instant::now();
        let failure = committer.commit("deliver d").wait().unwrap_err();
        let waited: u64 = RETRY_WAITS.iter().sum();
        assert!(
            started.elapsed() >= Duration::from_millis(waited),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(failure.subject, "deliver d");
        assert!(
            failure
                .reason
                .starts_with("the Agora's commit gave up after 5 attempts (subjects: deliver d): "),
            "{}",
            failure.reason
        );
        assert!(failure.reason.contains("index.lock"), "{}", failure.reason);
        assert!(root.join("m.json").exists(), "a failure loses no file");
        fs::remove_dir(&lock).unwrap();
        let outcome = committer.commit("deliver e").wait().unwrap();
        assert_eq!(outcome.attempts, 1);
        assert!(tracked(&root, "m.json"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_commit_git_refuses_after_staging_is_retried_then_given_up() {
        let (dir, root) = agora("commit-refused");
        let committer = open(&root);
        // Staging works; moving the branch cannot.
        let branch_lock = root
            .join(".git")
            .join("refs")
            .join("heads")
            .join("main.lock");
        write(&branch_lock, "");
        write(&root.join("m.json"), "{}\n");
        let failure = committer.commit("deliver").wait().unwrap_err();
        assert!(
            failure.reason.contains("after 5 attempts"),
            "{}",
            failure.reason
        );
        assert!(failure.reason.contains("main.lock"), "{}", failure.reason);
        fs::remove_file(&branch_lock).unwrap();
        assert_eq!(committer.commit("again").wait().unwrap().attempts, 1);
        assert!(tracked(&root, "m.json"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fault_aborts_its_batch_with_its_own_words_and_no_retry() {
        let (dir, root) = agora("fault");
        let armed = Arc::new(AtomicBool::new(true));
        let hooks = armed_at(
            FaultPoint::AfterStage,
            "blackout between stage and commit",
            &armed,
        );
        let committer = Committer::open(&root, hooks).unwrap().committer;
        write(&root.join("inbox").join("m1.json"), "{}\n");
        let failure = committer.commit("hermes: deliver 1").wait().unwrap_err();
        assert_eq!(
            failure,
            CommitFailure {
                subject: "hermes: deliver 1".to_string(),
                reason: "blackout between stage and commit".to_string(),
            }
        );
        assert!(
            !agora_git(&root, &["diff", "--cached", "--name-only"])
                .trim()
                .is_empty(),
            "the work was staged when the harness died"
        );
        assert!(committer.is_dirty());
        armed.store(false, Ordering::SeqCst);
        assert_eq!(committer.commit("next").wait().unwrap().attempts, 1);
        assert!(tracked(&root, "inbox/m1.json"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_fault_before_or_after_the_commit_aborts_its_batch_where_it_stands() {
        for (point, landed) in [
            (FaultPoint::BeforeCommit, false),
            (FaultPoint::AfterCommit, true),
        ] {
            let (dir, root) = agora("fault-around-commit");
            let committer = Committer::open(&root, once_at(point, "died"))
                .unwrap()
                .committer;
            write(&root.join("m.json"), "{}\n");
            let failure = committer.commit("deliver").wait().unwrap_err();
            assert_eq!(failure.reason, "died", "{point:?}");
            assert_eq!(
                tracked(&root, "m.json"),
                landed,
                "{point:?}: a harness dying after its commit has its work in history"
            );
            committer.commit("next").wait().unwrap();
            assert!(tracked(&root, "m.json"), "{point:?}");
            drop(committer);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_failure_nobody_waits_for_is_reported_kept_and_bounded() {
        let (dir, root) = agora("soon");
        let armed = Arc::new(AtomicBool::new(true));
        let reported = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&reported);
        let hooks = armed_at(FaultPoint::BeforeStage, "cannot commit", &armed)
            .reporting_to(move |failure| guard(&heard).push(failure.subject.clone()));
        let committer = Committer::open(&root, hooks).unwrap().committer;
        for n in 0..=MAX_RECORDED_FAILURES {
            committer.commit_soon(format!("s{n}"));
            committer.drained();
        }
        let failures = committer.failures();
        assert_eq!(failures.len(), MAX_RECORDED_FAILURES);
        assert_eq!(failures[0].subject, "s1", "the oldest is dropped first");
        assert_eq!(
            failures[MAX_RECORDED_FAILURES - 1].subject,
            format!("s{MAX_RECORDED_FAILURES}")
        );
        assert!(
            failures
                .iter()
                .all(|failure| failure.reason == "cannot commit")
        );
        assert_eq!(
            guard(&reported).len(),
            MAX_RECORDED_FAILURES + 1,
            "every one is reported"
        );
        // A ticket dropped unread is nobody waiting either.
        drop(committer.commit("dropped"));
        committer.drained();
        assert_eq!(committer.failures().last().unwrap().subject, "dropped");
        assert_eq!(guard(&reported).last().unwrap(), "dropped");
        armed.store(false, Ordering::SeqCst);
        write(&root.join("late.txt"), "late");
        committer.commit_soon("lands");
        committer.drained();
        assert_eq!(
            guard(&reported).len(),
            MAX_RECORDED_FAILURES + 2,
            "a landing reports nothing"
        );
        assert!(!committer.is_dirty());
        // A failure in the landing Drop waits for is reported too.
        armed.store(true, Ordering::SeqCst);
        committer.commit_soon("at quit");
        drop(committer);
        assert_eq!(guard(&reported).last().unwrap(), "at quit");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_drained_queue_commits_nothing_it_was_not_asked_to() {
        let (dir, root) = agora("drained");
        let committer = open(&root);
        write(&root.join("unasked.txt"), "x");
        committer.drained();
        assert!(committer.is_dirty());
        assert_eq!(commits(&root), 1);
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dropping_the_committer_lands_what_was_queued_first() {
        let (dir, root) = agora("drop");
        let committer = open(&root);
        write(&root.join("late.txt"), "late");
        committer.commit_soon("late");
        drop(committer);
        assert!(tracked(&root, "late.txt"));
        assert_eq!(
            agora_git(&root, &["log", "-1", "--format=%s"]).trim(),
            "late"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_removes_a_lock_that_outlived_the_schedule_and_reconcile_commits_the_rest() {
        let (dir, root) = agora("reconcile-stale");
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        write(&root.join("inbox").join("m1.json"), "{}\n");
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert_eq!(opened.index_lock, IndexLock::Removed);
        let outcome = opened.committer.reconcile().wait().unwrap();
        assert_eq!(outcome.subjects, [RECONCILE_SUBJECT]);
        assert_eq!(outcome.attempts, 1);
        assert!(!opened.committer.is_dirty());
        assert!(tracked(&root, "inbox/m1.json"));
        assert_eq!(
            agora_git(&root, &["log", "-1", "--format=%s"]).trim(),
            RECONCILE_SUBJECT
        );
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn open_lets_a_live_git_finish_before_any_commit_can_meet_its_lock() {
        let (dir, root) = agora("reconcile-live");
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        // The stand-in git finishes 30 ms in, far inside open's 375 ms wait. At
        // 200 ms, a slow Windows runner let open's own delete race this one, and
        // a file whose delete is still pending refuses a second with error 5
        // (CI, 2026-10-20): `Stuck`, which is the honest answer to that race.
        let opened = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(30));
                fs::remove_file(&lock).unwrap();
            });
            Committer::open(&root, Hooks::none()).unwrap()
        });
        assert_eq!(
            opened.index_lock,
            IndexLock::Absent,
            "the git finished on its own"
        );
        assert_eq!(opened.committer.reconcile().wait().unwrap().attempts, 1);
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn open_reports_a_lock_some_process_holds_open_on_windows() {
        let (dir, root) = agora("reconcile-held");
        // Git for Windows holds index.lock open without delete sharing while
        // it works (measured, 2026-10-20); `Exclusive` opens it the same way.
        let held = Exclusive::try_acquire(&root.join(".git").join("index.lock"))
            .unwrap()
            .unwrap();
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert!(
            matches!(&opened.index_lock, IndexLock::Stuck(_)),
            "{:?}",
            opened.index_lock
        );
        drop(held);
        drop(opened);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_commits_housekeeping_has_finished_when_drained_returns() {
        // The TypeScript build's test (agora.test.ts): the repository's own
        // config asks for detached housekeeping on every commit, so the
        // runner's `maintenance.autoDetach=false` must win, and one log-sized
        // blob of incompressible bytes gives the repack real work. Git for
        // Windows cannot detach, so this has teeth on POSIX only.
        let (dir, root) = agora("housekeeping");
        for (key, value) in [
            ("maintenance.auto", "true"),
            ("maintenance.autoDetach", "true"),
            ("maintenance.loose-objects.enabled", "true"),
            ("maintenance.loose-objects.auto", "-1"),
        ] {
            agora_git(&root, &["config", key, value]);
        }
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let noise: Vec<u8> = (0..4 * 1024 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect();
        write_atomic(&root.join("segment.bin"), &noise).unwrap();
        let committer = open(&root);
        committer.commit_soon("a log-sized blob for the housekeeping to pack");
        committer.drained();
        assert!(committer.failures().is_empty());
        let packs = root.join(".git").join("objects").join("pack");
        let packed = || {
            fs::read_dir(&packs).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.file_name().to_string_lossy().ends_with(".idx"))
            })
        };
        let packed_at_drained = packed();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut packed_ever = packed_at_drained;
        while !packed_ever && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
            packed_ever = packed();
        }
        assert!(
            packed_ever,
            "this git never ran the loose-objects task, so nothing was tested"
        );
        assert!(
            packed_at_drained,
            "drained returned while the commit's housekeeping was still packing"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_batch_message_names_every_subject_and_is_never_empty() {
        let subjects = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(commit_message(&[]), "agora update");
        assert_eq!(commit_message(&subjects(&["  "])), "agora update");
        assert_eq!(commit_message(&subjects(&["one"])), "one");
        assert_eq!(
            commit_message(&subjects(&["a", "b"])),
            "a (+1 more)\n\n- a\n- b"
        );
        assert_eq!(
            commit_message(&subjects(&["a", "b", "c"])),
            "a (+2 more)\n\n- a\n- b\n- c"
        );
    }

    #[test]
    fn a_ticket_whose_committer_stopped_says_so() {
        let (reply, receiver) = mpsc::channel();
        drop(reply);
        let ticket = Ticket {
            receiver,
            subject: "orphan".to_string(),
        };
        assert_eq!(ticket.wait().unwrap_err(), stopped("orphan".to_string()));
    }

    #[test]
    fn a_committer_whose_queue_is_gone_answers_instead_of_hanging() {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&reported);
        let gone = Committer {
            root: PathBuf::from("nowhere"),
            requests: None,
            worker: None,
            record: Arc::new(Record {
                kept: Mutex::new(VecDeque::new()),
                report: vec![Arc::new(move |failure: &CommitFailure| {
                    guard(&heard).push(failure.subject.clone());
                })],
            }),
        };
        assert_eq!(
            gone.commit("late").wait().unwrap_err(),
            stopped("late".to_string())
        );
        assert_eq!(
            gone.reconcile().wait().unwrap_err(),
            stopped(RECONCILE_SUBJECT.to_string())
        );
        assert_eq!(gone.head(), None);
        assert!(!gone.is_dirty());
        assert_eq!(
            gone.ignored().unwrap_err(),
            "the committer stopped before it answered"
        );
        gone.drained();
        gone.commit_soon("dropped");
        assert_eq!(gone.failures(), [stopped("dropped".to_string())]);
        assert_eq!(*guard(&reported), ["dropped"]);
        assert!(format!("{gone:?}").contains("kept: 1"), "{gone:?}");
    }

    /// Hooks recording every landing's subjects and ignored entries.
    fn hearing_landings() -> (Hooks, Arc<Mutex<Vec<Landing>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        let hooks = Hooks::none().on_landing(move |landing| guard(&into).push(landing.clone()));
        (hooks, heard)
    }

    #[test]
    fn every_landing_is_reported_before_its_tickets_are_answered() {
        let (dir, root) = agora("landing");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&heard);
        // A slow report: answered first, the ticket would return before it,
        // every time rather than when the thread loses a race.
        let hooks = Hooks::none().on_landing(move |landing| {
            thread::sleep(Duration::from_millis(200));
            guard(&into).push(landing.clone());
        });
        let committer = Committer::open(&root, hooks).unwrap().committer;
        write(&root.join("a.json"), "{}\n");
        committer.commit_soon("soon");
        committer.drained();
        assert_eq!(
            guard(&heard).len(),
            1,
            "a commit nobody waits for is heard landing"
        );
        assert_eq!(guard(&heard)[0].outcome.subjects, ["soon"]);
        write(&root.join("b.json"), "{}\n");
        let outcome = committer.commit("waited").wait().unwrap();
        let landings = guard(&heard).clone();
        assert_eq!(
            landings.last().map(|landing| &landing.outcome),
            Some(&outcome),
            "the report is made before the ticket is answered"
        );
        let nothing = committer.commit("nothing to commit").wait().unwrap();
        assert_eq!(
            guard(&heard).len(),
            3,
            "a batch that finds nothing to commit lands too"
        );
        assert_eq!(nothing.sha, outcome.sha);
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_batch_that_gives_up_is_not_a_landing_and_reporters_add_up() {
        let (dir, root) = agora("reporters");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let (first, second, third, fourth) = (
            Arc::clone(&heard),
            Arc::clone(&heard),
            Arc::clone(&heard),
            Arc::clone(&heard),
        );
        let armed = Arc::new(AtomicBool::new(true));
        let hooks = armed_at(FaultPoint::BeforeStage, "cannot commit", &armed)
            .reporting_to(move |failure| guard(&first).push(format!("1:{}", failure.subject)))
            .reporting_to(move |failure| guard(&second).push(format!("2:{}", failure.subject)))
            .on_landing(move |landing| {
                guard(&third).push(format!("3:{}", landing.outcome.subjects.join(",")));
            })
            .on_landing(move |landing| {
                guard(&fourth).push(format!("4:{}", landing.outcome.subjects.join(",")));
            });
        let committer = Committer::open(&root, hooks).unwrap().committer;
        committer.commit_soon("lost");
        committer.drained();
        assert_eq!(
            *guard(&heard),
            ["1:lost", "2:lost"],
            "both reporters hear, in order, and a failure is not a landing"
        );
        armed.store(false, Ordering::SeqCst);
        committer.commit("found").wait().unwrap();
        assert_eq!(
            *guard(&heard),
            ["1:lost", "2:lost", "3:found", "4:found"],
            "both landing reports hear, in order"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_landing_lists_what_git_ignores_and_a_batch_of_only_that_commits_nothing() {
        let (dir, root) = agora("ignored");
        write(&root.join(".gitignore"), "kept-out/\n*.log\n");
        let (hooks, heard) = hearing_landings();
        let committer = Committer::open(&root, hooks).unwrap().committer;
        committer.commit("ignore rules").wait().unwrap();
        let head = committer.head();
        write(&root.join("kept-out").join("x.json"), "{}\n");
        write(&root.join("a.log"), "log\n");
        write(
            &root.join("agents").join("a1").join(".gitignore"),
            "secret/\n",
        );
        write(
            &root
                .join("agents")
                .join("a1")
                .join("secret")
                .join("plan.md"),
            "hidden\n",
        );
        let outcome = committer
            .commit("an agent's own ignore file")
            .wait()
            .unwrap();
        assert_ne!(
            outcome.sha, head,
            "the agent's .gitignore itself is committed"
        );
        assert!(tracked(&root, "agents/a1/.gitignore"));
        let mut ignored = guard(&heard)
            .last()
            .unwrap()
            .ignored
            .clone()
            .unwrap()
            .entries;
        ignored.sort();
        assert_eq!(
            ignored,
            ["a.log", "agents/a1/secret/", "kept-out/"],
            "a directory a pattern matched is listed as the directory"
        );
        write(&root.join("b.log"), "more\n");
        let quiet = committer
            .commit("only ignored files changed")
            .wait()
            .unwrap();
        assert_eq!(
            quiet.sha, outcome.sha,
            "nothing to commit is still a landing"
        );
        assert!(
            guard(&heard)
                .last()
                .unwrap()
                .ignored
                .as_ref()
                .unwrap()
                .entries
                .contains(&"b.log".to_string())
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rename_s_source_path_is_not_read_as_an_entry() {
        let (dir, root) = agora("rename");
        // A source path that reads as an ignored entry if the parser takes it
        // for one: `! trap.json`.
        write(&root.join("! trap.json"), "{\"a\":1}\n");
        let (hooks, heard) = hearing_landings();
        let committer = Committer::open(&root, hooks).unwrap().committer;
        committer.commit("the trap").wait().unwrap();
        fs::rename(root.join("! trap.json"), root.join("moved.json")).unwrap();
        committer.commit("a move").wait().unwrap();
        assert!(tracked(&root, "moved.json"));
        assert_eq!(
            guard(&heard).last().unwrap().ignored,
            Ok(Ignored::default()),
            "a rename's source is one field of the entry, not an entry"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn fields(
        changed: bool,
        gitlinks: &[&str],
        ignored: &[&str],
        untracked: &[&str],
    ) -> StatusFields {
        let owned = |list: &[&str]| list.iter().map(|path| path.to_string()).collect();
        StatusFields {
            changed,
            gitlinks: owned(gitlinks),
            ignored: owned(ignored),
            untracked: owned(untracked),
        }
    }

    #[test]
    fn status_fields_are_read_whole() {
        let hash = "0".repeat(40);
        assert_eq!(read_status(""), StatusFields::default());
        assert_eq!(
            read_status("! a.log\0! dir/\0# branch.oid x\0"),
            fields(false, &[], &["a.log", "dir/"], &[]),
            "ignored entries and headers alone change nothing"
        );
        assert_eq!(
            read_status(&format!(
                "2 R. N... 100644 100644 100644 {hash} {hash} R100 new name.json\0! old.json\0! a.log\0"
            )),
            fields(true, &[], &["a.log"], &[]),
            "a rename's source is skipped, and a path keeps its spaces"
        );
        assert_eq!(
            read_status(&format!(
                "1 A. S... 000000 160000 160000 {hash} {hash} agents/a1/work dir\0"
            )),
            fields(true, &["agents/a1/work dir"], &[], &[]),
            "a staged gitlink, by its `sub` field"
        );
        assert_eq!(
            read_status(&format!(
                "2 R. S... 160000 160000 160000 {hash} {hash} R100 agents/a1/moved\0agents/a1/work\0"
            )),
            fields(true, &["agents/a1/moved"], &[], &[]),
            "a renamed gitlink, by its new path"
        );
        assert_eq!(
            read_status(&format!(
                "1 .M N... 100644 100644 100644 {hash} {hash} m.json\0! x.log\0"
            )),
            fields(true, &[], &["x.log"], &[]),
            "not every changed entry is a gitlink, and none has a source"
        );
        assert_eq!(
            read_status(&format!(
                "u UU N... 100644 100644 100644 100644 {hash} {hash} {hash} u.json\0? new.json\0? agents/a1/work/\0"
            )),
            fields(true, &[], &[], &["new.json", "agents/a1/work/"])
        );
        assert_eq!(
            read_status("!\0"),
            fields(false, &[], &[""], &[]),
            "a field too short to hold a path"
        );
    }

    #[test]
    fn a_kept_out_path_is_one_pattern_and_the_exclude_files_other_lines_stay() {
        for path in ["agents/a1/work", "agents/a*1/[x]?\\y", "agents/a 1/#!work"] {
            let pattern = escape_pattern(path);
            assert_eq!(unescape_pattern(&pattern), path, "{pattern}");
        }
        assert_eq!(escape_pattern("a*b?c[d\\e"), "a\\*b\\?c\\[d\\\\e");
        let kept = vec!["# the Architect's own".to_string(), "*.secret".to_string()];
        let nested = vec!["agents/a1/work".to_string(), "agents/a*2".to_string()];
        let text = exclude_text(&kept, &nested);
        assert_eq!(
            text,
            format!(
                "# the Architect's own\n*.secret\n{NESTED_BEGIN}\n/agents/a1/work/\n/agents/a\\*2/\n{NESTED_END}\n"
            )
        );
        assert_eq!(read_exclude(&text), (kept.clone(), nested));
        assert_eq!(
            exclude_text(&kept, &[]),
            "# the Architect's own\n*.secret\n",
            "an empty list writes no block"
        );
        assert_eq!(read_exclude(""), (Vec::new(), Vec::new()));
    }

    #[test]
    fn what_git_ignores_can_be_asked_without_staging_and_a_failed_status_says_so() {
        let (dir, root) = agora("ask-ignored");
        write(&root.join(".gitignore"), "*.log\n");
        let committer = open(&root);
        committer.commit("ignore rules").wait().unwrap();
        let head = committer.head();
        write(&root.join("a.log"), "log\n");
        write(&root.join("new.json"), "{}\n");
        assert_eq!(
            committer.ignored(),
            Ok(Ignored {
                entries: vec!["a.log".to_string()],
                nested: Vec::new()
            })
        );
        assert_eq!(committer.head(), head, "nothing was committed");
        assert!(!tracked(&root, "new.json"));
        write(&root.join(".git").join("index"), "not an index");
        let unknown = committer.ignored().unwrap_err();
        assert!(
            unknown.starts_with("git status failed, so what git ignores is unknown:"),
            "{unknown}"
        );
        assert!(
            committer.commit("over a broken index").wait().is_err(),
            "a status that fails is not a landing"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A repository an agent makes at `agents/<agent>/work`, with a file, and a
    /// commit when `committed`.
    fn nested_repo(root: &Path, agent: &str, committed: bool) -> PathBuf {
        let work = root.join("agents").join(agent).join("work");
        fs::create_dir_all(&work).unwrap();
        must(&work, &["init", "-q", "-b", "main"]);
        write(&work.join("plan.md"), "the plan\n");
        if committed {
            must(&work, &["add", "-A"]);
            must(
                &work,
                &[
                    "-c",
                    "user.name=A",
                    "-c",
                    "user.email=a@example.invalid",
                    "commit",
                    "-qm",
                    "inner",
                ],
            );
        }
        work
    }

    fn gitlinks(root: &Path) -> usize {
        agora_git(root, &["ls-tree", "-r", "HEAD"])
            .lines()
            .filter(|line| line.starts_with("160000"))
            .count()
    }

    /// DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, fourth round*,
    /// item 1: one agent's `git init` with no commit failed every `add -A`, every
    /// other agent's work included.
    #[test]
    fn a_nested_repository_with_no_commit_is_kept_out_and_the_rest_lands() {
        let (dir, root) = agora("nested-empty");
        nested_repo(&root, "a1", false);
        write(&root.join("agents/a2/outbox/m1.json"), "{}\n");
        let (hooks, heard) = hearing_landings();
        let committer = Committer::open(&root, hooks).unwrap().committer;
        let outcome = committer.commit("deliver").wait().unwrap();
        assert_eq!(
            outcome.attempts, 2,
            "the first attempt found it and kept it out"
        );
        assert!(
            tracked(&root, "agents/a2/outbox/m1.json"),
            "the rest landed"
        );
        assert!(!tracked(&root, "agents/a1/work/plan.md"));
        let ignored = guard(&heard).last().unwrap().ignored.clone().unwrap();
        assert_eq!(ignored.nested, ["agents/a1/work"]);
        assert!(
            ignored.entries.contains(&"agents/a1/work/".to_string()),
            "{ignored:?}"
        );
        let exclude = fs::read_to_string(root.join(".git").join(EXCLUDE_FILE)).unwrap();
        assert_eq!(
            exclude,
            format!("{NESTED_BEGIN}\n/agents/a1/work/\n{NESTED_END}\n")
        );
        drop(committer);
        // The list outlives the committer.
        let again = open(&root);
        write(&root.join("agents/a2/outbox/m2.json"), "{}\n");
        assert_eq!(again.commit("again").wait().unwrap().attempts, 1);
        assert_eq!(again.ignored().unwrap().nested, ["agents/a1/work"]);
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_nested_repository_with_a_commit_never_lands_as_a_gitlink_nor_wedges_a_batch() {
        let (dir, root) = agora("nested-gitlink");
        let work = nested_repo(&root, "a1", true);
        let (hooks, heard) = hearing_landings();
        let committer = Committer::open(&root, hooks).unwrap().committer;
        committer.commit("a batch").wait().unwrap();
        assert_eq!(
            gitlinks(&root),
            0,
            "its pointer was unstaged before the commit"
        );
        assert!(!committer.is_dirty());
        assert_eq!(
            guard(&heard)
                .last()
                .unwrap()
                .ignored
                .clone()
                .unwrap()
                .nested,
            ["agents/a1/work"]
        );
        // Dirty, and the batch's only change: once a wedge, now nothing to land.
        write(&work.join("plan.md"), "the plan, changed\n");
        let quiet = committer
            .commit("only the nested repository changed")
            .wait()
            .unwrap();
        assert_eq!(quiet.attempts, 1);
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_gitlink_already_in_history_is_taken_out_of_it_at_its_first_change() {
        let (dir, root) = agora("nested-in-head");
        let work = nested_repo(&root, "a1", true);
        // Committed by hand, as a build without this check would have.
        must(&root, &["add", "-A"]);
        must(
            &root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "-qm",
                "a gitlink",
            ],
        );
        assert_eq!(gitlinks(&root), 1);
        let committer = open(&root);
        // A clean gitlink shows in no status, so it stays until it changes,
        // and its files were never in history: no worse than before.
        committer.commit("nothing changed").wait().unwrap();
        assert_eq!(gitlinks(&root), 1);
        write(&work.join("plan.md"), "the plan, changed\n");
        committer.commit("its first change").wait().unwrap();
        assert_eq!(
            gitlinks(&root),
            0,
            "the next commit records the pointer's removal"
        );
        assert!(
            root.join("agents/a1/work/plan.md").is_file(),
            "the files stay on disk"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_directory_that_stops_being_a_repository_is_committed_again() {
        let (dir, root) = agora("nested-gone");
        let work = nested_repo(&root, "a1", false);
        write(&root.join(".git").join(EXCLUDE_FILE), "*.secret\n");
        let committer = open(&root);
        committer.commit("first").wait().unwrap();
        assert!(!tracked(&root, "agents/a1/work/plan.md"));
        let exclude = fs::read_to_string(root.join(".git").join(EXCLUDE_FILE)).unwrap();
        assert!(
            exclude.starts_with("*.secret\n"),
            "a line it did not write stays: {exclude}"
        );
        fs::remove_dir_all(work.join(".git")).unwrap();
        committer.commit("second").wait().unwrap();
        assert!(
            tracked(&root, "agents/a1/work/plan.md"),
            "its files are work again"
        );
        assert_eq!(committer.ignored().unwrap().nested, Vec::<String>::new());
        assert_eq!(
            fs::read_to_string(root.join(".git").join(EXCLUDE_FILE)).unwrap(),
            "*.secret\n",
            "an empty list leaves no block"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_list_that_cannot_be_written_fails_the_batch_and_says_why() {
        let (dir, root) = agora("nested-unwritable");
        nested_repo(&root, "a1", false);
        // `.git/info` is a file, so the exclude file cannot be made.
        write(&root.join(".git").join("info"), "not a directory");
        let committer = open(&root);
        let failure = committer.commit("blocked").wait().unwrap_err();
        assert!(
            failure
                .reason
                .contains("list of nested repositories could not be written"),
            "{}",
            failure.reason
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_list_with_nothing_to_prune_is_not_rewritten() {
        let (dir, root) = agora("nested-unchanged");
        nested_repo(&root, "a1", false);
        let committer = open(&root);
        committer.commit("kept out").wait().unwrap();
        let exclude = root.join(".git").join(EXCLUDE_FILE);
        let written = fs::metadata(&exclude).unwrap().modified().unwrap();
        thread::sleep(Duration::from_millis(50));
        write(&root.join("agents/a2/m.json"), "{}\n");
        committer.commit("another").wait().unwrap();
        assert_eq!(
            fs::metadata(&exclude).unwrap().modified().unwrap(),
            written,
            "a list whose every repository still has its .git is left alone"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_add_that_fails_for_another_reason_says_only_why() {
        let (dir, root) = agora("add-fails");
        let committer = open(&root);
        // A young lock is someone else's, so it is waited on, never removed.
        write(&root.join(".git").join("index.lock"), "");
        write(&root.join("m.json"), "{}\n");
        let failure = committer.commit("blocked").wait().unwrap_err();
        assert!(failure.reason.contains("index.lock"), "{}", failure.reason);
        assert!(
            !failure.reason.contains("kept out"),
            "no nested repository was found, so none is said to be kept out: {}",
            failure.reason
        );
        fs::remove_file(root.join(".git").join("index.lock")).unwrap();
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The written pattern, read by git itself: a `[` in the path is a
    /// wildcard unless escaped (`*` and `?` cannot be in a Windows file name).
    #[test]
    fn a_nested_repository_whose_path_holds_a_wildcard_is_kept_out_by_git_itself() {
        let (dir, root) = agora("nested-wildcard");
        nested_repo(&root, "a[1]", false);
        write(&root.join("agents/a1/work/m.json"), "{}\n");
        let committer = open(&root);
        let outcome = committer.commit("a batch").wait().unwrap();
        assert_eq!(outcome.attempts, 2);
        assert!(
            tracked(&root, "agents/a1/work/m.json"),
            "a path the unescaped pattern would have matched is committed"
        );
        assert!(!tracked(&root, "agents/a[1]/work/plan.md"));
        write(&root.join("agents/a2/m.json"), "{}\n");
        assert_eq!(
            committer.commit("again").wait().unwrap().attempts,
            1,
            "git reads the escaped pattern as the path it names"
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_list_that_cannot_be_kept_or_pruned_fails_the_batch_and_says_why() {
        // A gitlink to keep out, with nowhere to write the list.
        let (dir, root) = agora("nested-keep-blocked");
        nested_repo(&root, "a1", true);
        write(&root.join(".git").join("info"), "not a directory");
        let committer = open(&root);
        let failure = committer.commit("blocked").wait().unwrap_err();
        assert!(
            failure
                .reason
                .contains("list of nested repositories could not be written"),
            "{}",
            failure.reason
        );
        assert_eq!(
            committer.head(),
            Some(agora_git(&root, &["rev-parse", "HEAD"]).trim().to_string())
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
        // A repository kept out that stopped being one, with nowhere to write
        // the list without it.
        let (dir, root) = agora("nested-prune-blocked");
        let work = nested_repo(&root, "a1", false);
        let committer = open(&root);
        committer.commit("kept out").wait().unwrap();
        fs::remove_dir_all(work.join(".git")).unwrap();
        fs::remove_dir_all(root.join(".git").join("info")).unwrap();
        write(&root.join(".git").join("info"), "not a directory");
        let failure = committer.commit("blocked").wait().unwrap_err();
        assert!(
            failure
                .reason
                .contains("list of nested repositories could not be written"),
            "{}",
            failure.reason
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_gitlink_that_cannot_be_unstaged_at_first_is_unstaged_by_the_retry() {
        let (dir, root) = agora("nested-unstage-retry");
        nested_repo(&root, "a1", true);
        let lock = root.join(".git").join("index.lock");
        let stages = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&stages);
        let planted = lock.clone();
        // The first attempt's `rm --cached` meets a lock it cannot take; status,
        // which takes no optional lock, still reads. The retry finds it gone.
        let hooks = Hooks::none().with_faults(move |point| {
            match point {
                FaultPoint::AfterStage if seen.fetch_add(1, Ordering::SeqCst) == 0 => {
                    write(&planted, "");
                }
                FaultPoint::BeforeStage if planted.exists() => fs::remove_file(&planted).unwrap(),
                _ => {}
            }
            Ok(())
        });
        let committer = Committer::open(&root, hooks).unwrap().committer;
        let outcome = committer.commit("a batch").wait().unwrap();
        assert_eq!(outcome.attempts, 2, "the first could not unstage it");
        assert_eq!(gitlinks(&root), 0);
        assert!(!lock.exists());
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    // The kill tests. Each starts this test binary again, as a harness running
    // `child` below, and kills it for real: by its own exit at a fault point,
    // between git calls, or by the OS at random moments of a commit loop, which
    // may or may not land inside a git. Either way they assert the outcome on
    // reopen, not where the kill landed.

    #[expect(
        clippy::disallowed_methods,
        reason = "process: the kill tests run this test binary again, as the harness they kill"
    )]
    fn spawn_child(root: &Path, mode: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "committer::tests::child",
                "--test-threads=1",
                "--quiet",
            ])
            .env(CHILD_AGORA, root)
            .env(CHILD_MODE, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Waits for `child` to exit, killing it if it outlives [`CHILD_PATIENCE`].
    fn exit_of(mut child: Child) -> Option<i32> {
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
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Not a test of its own: the body of the child process the kill tests
    /// start. Run without their variables, it does nothing.
    #[test]
    fn child() {
        let (Some(root), Ok(mode)) = (std::env::var_os(CHILD_AGORA), std::env::var(CHILD_MODE))
        else {
            return;
        };
        let root = PathBuf::from(root);
        match mode.as_str() {
            "hold" => {
                let committer = open(&root);
                write(&root.with_file_name("held"), "");
                thread::sleep(Duration::from_secs(60));
                drop(committer);
            }
            "churn" => {
                let committer = open(&root);
                let work = root.join("work");
                let mut n = fs::read_dir(&work).map_or(0, Iterator::count);
                loop {
                    n += 1;
                    write(&work.join(format!("{n:06}.txt")), &n.to_string());
                    let _landed = committer.commit(format!("work {n}")).wait();
                }
            }
            point => {
                let point = match point {
                    "before-stage" => FaultPoint::BeforeStage,
                    "after-stage" => FaultPoint::AfterStage,
                    "before-commit" => FaultPoint::BeforeCommit,
                    _ => FaultPoint::AfterCommit,
                };
                let hooks = Hooks::none().with_faults(move |at| {
                    if at == point {
                        std::process::exit(DIED);
                    }
                    Ok(())
                });
                let committer = Committer::open(&root, hooks).unwrap().committer;
                let _never = committer.commit("killed").wait();
            }
        }
    }

    #[test]
    fn a_harness_killed_at_each_point_of_a_commit_loses_nothing() {
        for point in [
            "before-stage",
            "after-stage",
            "before-commit",
            "after-commit",
        ] {
            let (dir, root) = agora(&format!("killed-{point}"));
            write(&root.join("inbox").join("m1.json"), "{}\n");
            assert_eq!(
                exit_of(spawn_child(&root, point)),
                Some(DIED),
                "{point}: the child died at its point"
            );
            let opened = Committer::open(&root, Hooks::none()).unwrap();
            assert_eq!(opened.index_lock, IndexLock::Absent, "{point}");
            opened.committer.reconcile().wait().unwrap();
            assert!(!opened.committer.is_dirty(), "{point}");
            assert!(tracked(&root, "inbox/m1.json"), "{point}");
            drop(opened);
            agora_git(&root, &["fsck", "--no-dangling"]);
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn a_harness_killed_at_random_during_commits_loses_nothing() {
        let (dir, root) = agora("killed-random");
        for after in [150, 300, 500, 800] {
            let mut child = spawn_child(&root, "churn");
            thread::sleep(Duration::from_millis(after));
            child.kill().unwrap();
            let _reaped = child.wait().unwrap();
        }
        // A write the kill cut short leaves a `.<name>.<hex>.tmp` beside its
        // target (R1.2's residual). Only finished files are work.
        let finished = |names: Vec<String>| {
            names
                .into_iter()
                .filter(|name| !name.rsplit('/').next().unwrap_or(name).starts_with('.'))
                .count()
        };
        let written = finished(
            fs::read_dir(root.join("work"))
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect(),
        );
        assert!(written > 0, "the children wrote work to lose");
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        opened.committer.reconcile().wait().unwrap();
        assert!(!opened.committer.is_dirty());
        let committed = finished(
            agora_git(&root, &["ls-files", "work"])
                .lines()
                .map(str::to_string)
                .collect(),
        );
        assert_eq!(
            committed, written,
            "every file a killed harness finished is committed"
        );
        drop(opened);
        agora_git(&root, &["fsck", "--no-dangling"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_killed_committer_releases_its_lock() {
        let (dir, root) = agora("killed-holder");
        let marker = root.with_file_name("held");
        let mut child = spawn_child(&root, "hold");
        let deadline = Instant::now() + CHILD_PATIENCE;
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let held = Committer::open(&root, Hooks::none());
        child.kill().unwrap();
        let _reaped = child.wait().unwrap();
        assert!(marker.exists(), "the child never took the lock");
        assert!(matches!(held, Err(OpenError::Held(_))), "{held:?}");
        drop(open(&root));
        fs::remove_dir_all(&dir).unwrap();
    }
}

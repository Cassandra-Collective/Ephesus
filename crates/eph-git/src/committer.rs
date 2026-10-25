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
//! - **Dropping the committer lands its queue first**, then releases the lock.
//!   That can take as long as the queued gits do, each bounded by its deadline.
//!
//! The Agora's own files (`PROTOCOL.md`, the registry, the ledger, the board,
//! the book of record) are R1.4's. [`Opened::initialized`] tells that boot
//! whether this open made the repository.

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

/// What a caller hangs on the committer. Production passes a reporter, for the
/// degradation channel (R1.4); tests add faults. The fault seam is in the
/// production path on purpose, because a seam that exists only under a mock
/// proves nothing about the real order of the steps (TEST-STRATEGY §3).
#[derive(Clone)]
pub struct Hooks {
    faults: Option<Arc<FaultHook>>,
    report: Option<Arc<ReportHook>>,
}

impl Hooks {
    /// No faults and no reporter.
    pub fn none() -> Hooks {
        Hooks {
            faults: None,
            report: None,
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

    /// The same hooks reporting every failure nobody waited for to `hook`, on
    /// the committer's thread. Contract: `hook` must not queue a commit. The
    /// failing path is git itself, and a reporter that commits would recurse.
    pub fn reporting_to(mut self, hook: impl Fn(&CommitFailure) + Send + Sync + 'static) -> Hooks {
        self.report = Some(Arc::new(hook));
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
        if initialized {
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
    report: Option<Arc<ReportHook>>,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("kept", &guard(&self.kept).len())
            .field("reported", &self.report.is_some())
            .finish()
    }
}

impl Record {
    /// Reports `failure`, then keeps it, dropping the oldest past
    /// [`MAX_RECORDED_FAILURES`]. The reporter runs before the lock is taken,
    /// so it may read [`Committer::failures`].
    fn keep(&self, failure: CommitFailure) {
        if let Some(report) = &self.report {
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
}

/// How one attempt ended.
enum Attempt {
    Landed,
    Failed(String),
    Aborted(String),
}

/// The thread that owns the Agora's git and the committer's lock.
struct Worker {
    root: PathBuf,
    git: Git,
    hooks: Hooks,
    record: Arc<Record>,
    _lock: Exclusive,
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
        let landed = self.run_commit(subjects);
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

    fn run_commit(&self, subjects: Vec<String>) -> Result<CommitOutcome, String> {
        let message = commit_message(&subjects);
        let mut last = String::new();
        let waits = iter::once(None).chain(RETRY_WAITS.map(Some));
        for (attempt, wait) in (1..).zip(waits) {
            if let Some(wait) = wait {
                thread::sleep(Duration::from_millis(wait));
                self.clear_stale_index_lock();
            }
            match self.attempt(&message) {
                Attempt::Landed => {
                    return Ok(CommitOutcome {
                        subjects,
                        sha: self.head(),
                        attempts: attempt,
                    });
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
        let staged = self.git(&["add", "-A"]);
        if !staged.ok() {
            return Attempt::Failed(staged.stderr.trim().to_string());
        }
        if let Err(reason) = self.hooks.check(FaultPoint::AfterStage) {
            return Attempt::Aborted(reason);
        }
        // Nothing staged is a landing; a status that fails is not, and the
        // commit after it decides.
        let status = self.git(&["status", "--porcelain"]);
        if status.ok() && status.stdout.trim().is_empty() {
            return Attempt::Landed;
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
        Attempt::Landed
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
    use std::sync::atomic::{AtomicBool, Ordering};
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
        // The lock came first, and the next open finishes the repository.
        assert!(root.join(".git").join(LOCK_FILE).exists());
        let opened = Committer::open(&root, Hooks::none()).unwrap();
        assert!(opened.initialized);
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
                report: Some(Arc::new(move |failure: &CommitFailure| {
                    guard(&heard).push(failure.subject.clone());
                })),
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
        gone.drained();
        gone.commit_soon("dropped");
        assert_eq!(gone.failures(), [stopped("dropped".to_string())]);
        assert_eq!(*guard(&reported), ["dropped"]);
        assert!(format!("{gone:?}").contains("kept: 1"), "{gone:?}");
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

//! `Committer`: the Agora's single committer (ADR-0004; FR-4.1; invariant 4).
//!
//! Contract:
//! - **One committer per Agora, enforced.** [`Committer::open`] takes an OS lock
//!   on `.git/ephesus-committer.lock` and holds it until the committer is
//!   dropped and its last git has finished. A second committer on the same
//!   Agora, in this process or another, is refused with [`OpenError::Held`].
//!   The OS releases the lock when the process dies, however it dies
//!   (DECISIONS-LOG 2026-10-20, *R1.3 — Architect decisions*).
//! - **One queue.** One thread runs every git call on the Agora, so two can never
//!   run at once and `index.lock` contention cannot be self-inflicted.
//! - **Batching.** Commits enqueued while the thread is busy land together, as
//!   one commit whose message names every subject. Delivery is a rename,
//!   durability is a commit, and commit throughput stays off delivery's path.
//! - **Retries live here and nowhere else** (BUILD-PROMPT §7). A failed git
//!   call is retried, five attempts in all, over the fixed [`RETRY_WAITS`], and
//!   a stale `index.lock` is cleared before each retry. A lock can only be
//!   stale here: this committer's own lock is held, so no other committer runs,
//!   and none of its own gits does.
//! - **A fault aborts its batch, with no retry.** A [`Faults`] hook that refuses
//!   at a [`FaultPoint`] stands for the harness dying there (TEST-STRATEGY §3,
//!   S-BLACKOUT). Its reason is the batch's failure, word for word.
//! - **Reconcile after a restart.** [`Committer::reconcile`] gives a git still
//!   running for a killed harness the retry schedule to finish, removes
//!   `index.lock` if it outlived it, and commits whatever was left.
//! - **A failure loses no work.** A commit that gives up leaves the files on
//!   disk, correct and not yet in history, and the next commit picks them up. A
//!   failure nobody waits for is kept ([`Committer::failures`], the latest
//!   [`MAX_RECORDED_FAILURES`]) instead of vanishing (invariant 7).
//! - **Dropping the committer lands its queue first**, then releases the lock.
//!
//! The Agora's own files (`PROTOCOL.md`, the registry, the ledger, the board,
//! the book of record) are R1.4's. [`Opened::initialized`] tells that boot
//! whether this open made the repository.

use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io;
use std::iter;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eph_fs::lock::Exclusive;

use crate::runner::{Git, GitResult};

/// The committer's lock file, inside the Agora's `.git`, so no `add -A` sees it.
pub const LOCK_FILE: &str = "ephesus-committer.lock";

/// The waits before each retry, in milliseconds: five attempts in all, 25 ms
/// doubling (today's figures), as a fixed schedule that cannot run forever.
pub const RETRY_WAITS: [u64; 4] = [25, 50, 100, 200];

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

type Hook = dyn Fn(FaultPoint) -> Result<(), String> + Send + Sync;

/// A fault hook: `Err(reason)` at a point aborts that batch with `reason`.
/// Production passes [`Faults::none`]. The seam is in the production path on
/// purpose, because a seam that exists only under a mock proves nothing about
/// the real order of the steps (TEST-STRATEGY §3).
#[derive(Clone, Default)]
pub struct Faults(Option<Arc<Hook>>);

impl Faults {
    /// No faults: what production runs.
    pub fn none() -> Faults {
        Faults(None)
    }

    /// Faults decided by `hook` at every point.
    pub fn at(hook: impl Fn(FaultPoint) -> Result<(), String> + Send + Sync + 'static) -> Faults {
        Faults(Some(Arc::new(hook)))
    }

    fn check(&self, point: FaultPoint) -> Result<(), String> {
        match &self.0 {
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

/// What a reconcile found at `.git/index.lock`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexLock {
    /// There was none, or the git holding it finished within the schedule.
    Absent,
    /// One outlived the schedule and was removed.
    Removed,
    /// One outlived the schedule and could not be removed: on Windows, a git
    /// still holds it. The commit's own retries come back to it.
    Held(String),
}

/// What a reconcile did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// The state of `index.lock` before the commit.
    pub index_lock: IndexLock,
    /// The commit of whatever a killed harness left behind.
    pub commit: Result<CommitOutcome, CommitFailure>,
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
#[derive(Debug)]
pub struct Opened {
    /// Its committer.
    pub committer: Committer,
    /// Whether this open ran `git init`.
    pub initialized: bool,
}

/// The answer to one [`Committer::commit`].
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
    failures: Arc<Mutex<VecDeque<CommitFailure>>>,
}

impl Committer {
    /// Opens the Agora at `root`: makes the directory and runs `git init -b
    /// main` when there is no `.git`, takes the committer's lock, and starts
    /// the queue. `faults` is [`Faults::none`] in production.
    pub fn open(root: &Path, faults: Faults) -> Result<Opened, OpenError> {
        Committer::open_with(root, faults, Git::agora())
    }

    /// [`Committer::open`] with a runner a test chose.
    pub(crate) fn open_with(root: &Path, faults: Faults, git: Git) -> Result<Opened, OpenError> {
        fs::create_dir_all(root)?;
        let dot_git = root.join(".git");
        let initialized = !dot_git.exists();
        if initialized {
            let init = git.run(root, &["init", "-b", "main"]);
            if !init.ok() {
                return Err(OpenError::Init(init.stderr.trim().to_string()));
            }
        } else if !dot_git.is_dir() {
            return Err(OpenError::NotARepository(dot_git));
        }
        let lock_path = dot_git.join(LOCK_FILE);
        let Some(lock) = Exclusive::try_acquire(&lock_path)? else {
            return Err(OpenError::Held(lock_path));
        };
        let failures = Arc::new(Mutex::new(VecDeque::new()));
        let worker = Worker {
            root: root.to_path_buf(),
            git,
            faults,
            failures: Arc::clone(&failures),
            _lock: lock,
        };
        let (sender, receiver) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("eph-git-committer".to_string())
            .spawn(move || worker.serve(&receiver))?;
        Ok(Opened {
            committer: Committer {
                root: root.to_path_buf(),
                requests: Some(sender),
                worker: Some(handle),
                failures,
            },
            initialized,
        })
    }

    /// The Agora's root.
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

    /// Queues a commit nobody waits for. Its failure is kept in
    /// [`Committer::failures`].
    pub fn commit_soon(&self, subject: impl Into<String>) {
        self.send(Request::Commit(Pending {
            subject: subject.into(),
            reply: None,
        }));
    }

    /// Clears a stale `index.lock` and commits what a killed harness left. See
    /// the module contract.
    pub fn reconcile(&self) -> Reconciled {
        self.ask(Query::Reconcile, || Reconciled {
            index_lock: IndexLock::Absent,
            commit: Err(stopped(RECONCILE_SUBJECT.to_string())),
        })
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
        guard(&self.failures).iter().cloned().collect()
    }

    /// Queues `request`. One the queue cannot take is answered as stopped: a
    /// ticket's sender is dropped with it, and a commit nobody waits for is
    /// recorded as a failure rather than lost.
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
            record(&self.failures, stopped(subject));
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

fn stopped(subject: String) -> CommitFailure {
    CommitFailure {
        subject,
        reason: "the committer stopped before it answered".to_string(),
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps `failure`, dropping the oldest past [`MAX_RECORDED_FAILURES`].
fn record(failures: &Mutex<VecDeque<CommitFailure>>, failure: CommitFailure) {
    let mut failures = guard(failures);
    failures.push_back(failure);
    while failures.len() > MAX_RECORDED_FAILURES {
        failures.pop_front();
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
    Reconcile(Sender<Reconciled>),
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
    faults: Faults,
    failures: Arc<Mutex<VecDeque<CommitFailure>>>,
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
            Query::Reconcile(reply) => {
                let _sent = reply.send(self.reconcile());
            }
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
            match (pending.reply, result) {
                (Some(reply), result) => {
                    let _sent = reply.send(result);
                }
                (None, Err(failure)) => record(&self.failures, failure),
                (None, Ok(_)) => {}
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
                self.clear_index_lock();
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
        if let Err(reason) = self.faults.check(FaultPoint::BeforeStage) {
            return Attempt::Aborted(reason);
        }
        let staged = self.git(&["add", "-A"]);
        if !staged.ok() {
            return Attempt::Failed(staged.stderr.trim().to_string());
        }
        if let Err(reason) = self.faults.check(FaultPoint::AfterStage) {
            return Attempt::Aborted(reason);
        }
        // Nothing staged is a landing; a status that fails is not, and the
        // commit after it decides.
        let status = self.git(&["status", "--porcelain"]);
        if status.ok() && status.stdout.trim().is_empty() {
            return Attempt::Landed;
        }
        if let Err(reason) = self.faults.check(FaultPoint::BeforeCommit) {
            return Attempt::Aborted(reason);
        }
        let committed = self.git(&["commit", "-m", message]);
        if !committed.ok() {
            return Attempt::Failed(committed.stderr.trim().to_string());
        }
        if let Err(reason) = self.faults.check(FaultPoint::AfterCommit) {
            return Attempt::Aborted(reason);
        }
        Attempt::Landed
    }

    fn reconcile(&self) -> Reconciled {
        let index_lock = self.settle_index_lock();
        let commit = self
            .run_commit(vec![RECONCILE_SUBJECT.to_string()])
            .map_err(|reason| CommitFailure {
                subject: RECONCILE_SUBJECT.to_string(),
                reason,
            });
        Reconciled { index_lock, commit }
    }

    /// Waits while `index.lock` exists, over the retry schedule, so a git left
    /// running by a killed harness can finish; then removes one that outlived it.
    fn settle_index_lock(&self) -> IndexLock {
        let lock = self.index_lock();
        let mut waits = RETRY_WAITS.iter();
        while lock.exists() {
            match waits.next() {
                Some(wait) => thread::sleep(Duration::from_millis(*wait)),
                None => return self.clear_index_lock(),
            }
        }
        IndexLock::Absent
    }

    fn clear_index_lock(&self) -> IndexLock {
        match fs::remove_file(self.index_lock()) {
            Ok(()) => IndexLock::Removed,
            Err(error) if error.kind() == io::ErrorKind::NotFound => IndexLock::Absent,
            Err(error) => IndexLock::Held(error.to_string()),
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

    fn open(root: &Path) -> Committer {
        Committer::open(root, Faults::none()).unwrap().committer
    }

    /// Faults that refuse once, at `at`, with `reason`.
    fn once_at(at: FaultPoint, reason: &'static str) -> Faults {
        let armed = AtomicBool::new(true);
        Faults::at(move |point| {
            if point == at && armed.swap(false, Ordering::SeqCst) {
                return Err(reason.to_string());
            }
            Ok(())
        })
    }

    #[test]
    fn a_second_committer_through_a_link_is_refused_too() {
        let (dir, root) = agora("second-link");
        let first = open(&root);
        let link = dir.join("via-link");
        link_dir(&root, &link);
        assert!(matches!(
            Committer::open(&link, Faults::none()),
            Err(OpenError::Held(_))
        ));
        drop(first);
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
    fn a_committer_whose_queue_is_gone_answers_instead_of_hanging() {
        let gone = Committer {
            root: PathBuf::from("nowhere"),
            requests: None,
            worker: None,
            failures: Arc::default(),
        };
        assert_eq!(
            gone.commit("late").wait().unwrap_err(),
            stopped("late".to_string())
        );
        let reconciled = gone.reconcile();
        assert_eq!(reconciled.index_lock, IndexLock::Absent);
        assert_eq!(
            reconciled.commit.unwrap_err(),
            stopped(RECONCILE_SUBJECT.to_string())
        );
        assert_eq!(gone.head(), None);
        assert!(!gone.is_dirty());
        gone.drained();
        gone.commit_soon("dropped");
        assert_eq!(gone.failures(), [stopped("dropped".to_string())]);
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

    #[test]
    fn a_new_agora_is_made_on_main_and_committed_as_ephesus() {
        let dir = scratch("new-agora");
        let root = dir.join("agora");
        let opened = Committer::open(&root, Faults::none()).unwrap();
        assert!(opened.initialized);
        let committer = opened.committer;
        assert_eq!(committer.root(), root);
        assert_eq!(committer.head(), None, "no commit yet");
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
        let again = Committer::open(&root, Faults::none()).unwrap();
        assert!(!again.initialized, "a second open makes nothing");
        assert_eq!(commits(&root), 1);
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_committer_is_refused_until_the_first_is_gone() {
        let (dir, root) = agora("second");
        let first = open(&root);
        let refused = Committer::open(&root, Faults::none()).unwrap_err();
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
    fn a_dot_git_that_is_not_a_directory_is_refused() {
        let dir = scratch("dot-git-file");
        let root = dir.join("agora");
        write(&root.join(".git"), "gitdir: elsewhere\n");
        let refused = Committer::open(&root, Faults::none()).unwrap_err();
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
        let io = Committer::open(&file.join("agora"), Faults::none()).unwrap_err();
        assert!(matches!(io, OpenError::Io(_)), "{io:?}");
        assert!(
            io.to_string()
                .starts_with("the Agora could not be opened: "),
            "{io}"
        );
        let refusing = host_with(&[("GIT_DEFAULT_HASH", "nonsense")]);
        let init = Committer::open_with(
            &dir.join("agora"),
            Faults::none(),
            Git::agora().with_environment(refusing),
        )
        .unwrap_err();
        assert!(matches!(init, OpenError::Init(_)), "{init:?}");
        let said = init.to_string();
        assert!(said.starts_with("git init failed: "), "{said}");
        assert!(said.contains("nonsense"), "in git's words: {said}");
        assert!(!dir.join("agora").join(".git").join(LOCK_FILE).exists());
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
            Committer::open_with(&root, Faults::none(), Git::agora().with_environment(host))
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
        let faults = Faults::at(move |point| {
            if point == FaultPoint::AfterCommit && first.swap(false, Ordering::SeqCst) {
                guard(&entered).send(()).unwrap();
                let _resumed = guard(&released).recv();
            }
            Ok(())
        });
        let committer = Committer::open(&root, faults).unwrap().committer;
        write(&root.join("a.txt"), "a");
        let a = committer.commit("first");
        parked.recv().unwrap();
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
    fn many_callers_at_once_never_contend_for_the_index() {
        let (dir, root) = agora("many");
        let committer = open(&root);
        thread::scope(|scope| {
            for n in 0..8 {
                let committer = &committer;
                let root = &root;
                scope.spawn(move || {
                    write(&root.join("work").join(format!("{n}.txt")), "w");
                    let outcome = committer.commit(format!("work {n}")).wait().unwrap();
                    assert_eq!(outcome.attempts, 1, "one queue: no lock to retry on");
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
    fn a_stale_index_lock_is_cleared_by_the_retry_and_the_commit_lands() {
        let (dir, root) = agora("stale-retry");
        let committer = open(&root);
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        write(&root.join("m.json"), "{}\n");
        let outcome = committer.commit("after a crash").wait().unwrap();
        assert_eq!(outcome.attempts, 2, "the first attempt met the lock");
        assert!(!lock.exists());
        assert!(tracked(&root, "m.json"));
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_commit_that_keeps_failing_gives_up_naming_its_subjects_and_the_queue_goes_on() {
        let (dir, root) = agora("gives-up");
        let committer = open(&root);
        // A directory where git makes its lock: no attempt can stage, and no
        // retry can clear it.
        let lock = root.join(".git").join("index.lock");
        fs::create_dir_all(&lock).unwrap();
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
    fn a_fault_aborts_its_batch_with_its_own_words_and_no_retry() {
        let (dir, root) = agora("fault");
        let armed = Arc::new(AtomicBool::new(true));
        let hook = Arc::clone(&armed);
        let faults = Faults::at(move |point| {
            if point == FaultPoint::AfterStage && hook.load(Ordering::SeqCst) {
                return Err("blackout between stage and commit".to_string());
            }
            Ok(())
        });
        let committer = Committer::open(&root, faults).unwrap().committer;
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
    fn a_failure_nobody_waits_for_is_kept_and_the_record_is_bounded() {
        let (dir, root) = agora("soon");
        let armed = Arc::new(AtomicBool::new(true));
        let hook = Arc::clone(&armed);
        let faults = Faults::at(move |point| {
            if point == FaultPoint::BeforeStage && hook.load(Ordering::SeqCst) {
                return Err("cannot commit".to_string());
            }
            Ok(())
        });
        let committer = Committer::open(&root, faults).unwrap().committer;
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
        armed.store(false, Ordering::SeqCst);
        write(&root.join("late.txt"), "late");
        committer.commit_soon("lands");
        committer.drained();
        assert_eq!(
            committer.failures().len(),
            MAX_RECORDED_FAILURES,
            "a landing records nothing"
        );
        assert!(!committer.is_dirty());
        drop(committer);
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
    fn reconcile_removes_a_lock_that_outlived_the_schedule_and_commits_what_was_left() {
        let (dir, root) = agora("reconcile-stale");
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        write(&root.join("inbox").join("m1.json"), "{}\n");
        let committer = open(&root);
        let reconciled = committer.reconcile();
        assert_eq!(reconciled.index_lock, IndexLock::Removed);
        let outcome = reconciled.commit.unwrap();
        assert_eq!(outcome.subjects, [RECONCILE_SUBJECT]);
        assert_eq!(outcome.attempts, 1);
        assert!(!committer.is_dirty());
        assert!(tracked(&root, "inbox/m1.json"));
        assert_eq!(
            agora_git(&root, &["log", "-1", "--format=%s"]).trim(),
            RECONCILE_SUBJECT
        );
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reconcile_lets_a_live_git_finish_instead_of_removing_its_lock() {
        let (dir, root) = agora("reconcile-live");
        let lock = root.join(".git").join("index.lock");
        write(&lock, "");
        let committer = open(&root);
        let reconciled = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(60));
                fs::remove_file(&lock).unwrap();
            });
            committer.reconcile()
        });
        assert_eq!(
            reconciled.index_lock,
            IndexLock::Absent,
            "the git finished on its own"
        );
        assert_eq!(reconciled.commit.unwrap().attempts, 1);
        assert_eq!(committer.reconcile().index_lock, IndexLock::Absent);
        drop(committer);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn reconcile_reports_a_lock_a_git_still_holds_on_windows() {
        let (dir, root) = agora("reconcile-held");
        let held = Exclusive::try_acquire(&root.join(".git").join("index.lock"))
            .unwrap()
            .unwrap();
        let committer = open(&root);
        let reconciled = committer.reconcile();
        assert!(
            matches!(&reconciled.index_lock, IndexLock::Held(_)),
            "{reconciled:?}"
        );
        assert_eq!(reconciled.commit.unwrap_err().subject, RECONCILE_SUBJECT);
        drop(held);
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

    // The kill tests. Each starts this test binary again, as a harness running
    // `child` below, and kills it for real: by its own exit at a fault point,
    // or by the OS at a random moment of a commit loop.

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
                let faults = Faults::at(move |at| {
                    if at == point {
                        std::process::exit(DIED);
                    }
                    Ok(())
                });
                let committer = Committer::open(&root, faults).unwrap().committer;
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
            let status = spawn_child(&root, point).wait().unwrap();
            assert_eq!(
                status.code(),
                Some(DIED),
                "{point}: the child died at its point"
            );
            let committer = open(&root);
            let reconciled = committer.reconcile();
            assert_eq!(reconciled.index_lock, IndexLock::Absent, "{point}");
            reconciled.commit.unwrap();
            assert!(!committer.is_dirty(), "{point}");
            assert!(tracked(&root, "inbox/m1.json"), "{point}");
            drop(committer);
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
        let written = fs::read_dir(root.join("work")).map_or(0, Iterator::count);
        assert!(written > 0, "the children wrote work to lose");
        let committer = open(&root);
        committer.reconcile().commit.unwrap();
        assert!(!committer.is_dirty());
        let committed = agora_git(&root, &["ls-files", "work"]).lines().count();
        assert_eq!(
            committed, written,
            "every file a killed harness wrote is committed"
        );
        drop(committer);
        agora_git(&root, &["fsck", "--no-dangling"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_killed_committer_releases_its_lock() {
        let (dir, root) = agora("killed-holder");
        let marker = root.with_file_name("held");
        let mut child = spawn_child(&root, "hold");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "the child never took the lock");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(
            Committer::open(&root, Faults::none()),
            Err(OpenError::Held(_))
        ));
        child.kill().unwrap();
        let _reaped = child.wait().unwrap();
        drop(open(&root));
        fs::remove_dir_all(&dir).unwrap();
    }
}

//! `Git`: the one place a git process starts (invariant 4; the `git:` exemption
//! RUST-PLAN §3.2 gives this crate alone).
//!
//! Contract: [`Git::run`] runs `git <overrides> <args>` in `cwd` and returns a
//! [`GitResult`]. It never panics and never returns an error. A git that could
//! not start, failed, overran its deadline, or whose output never arrived is a
//! result with no `code`, and its `stderr` ends with a sentence saying which.
//!
//! **Every call**, whatever its profile:
//! - takes no input (stdin is null) and asks nothing (`GIT_TERMINAL_PROMPT=0`),
//!   so a credential prompt can never hang the harness;
//! - takes no optional lock (`GIT_OPTIONAL_LOCKS=0`), so `status` never holds
//!   `index.lock`, and reads a pathspec literally (`GIT_LITERAL_PATHSPECS=1`);
//! - drops the variables that point git at a repository (`GIT_DIR`,
//!   `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_QUARANTINE_PATH`, …). A harness
//!   started from a git hook or an agent's shell would otherwise run against
//!   someone else's repository;
//! - never signs and never runs a hook, and the housekeeping a commit starts
//!   runs in the foreground, inside the call (`maintenance.autoDetach=false`,
//!   `gc.autoDetach=false`; memory: git's housekeeping outlives the commit);
//! - has its output read on two threads while the deadline is watched, so a git
//!   that fills a pipe is never stalled;
//! - opens no console window on Windows.
//!
//! **The deadline (20 s) triggers owned shutdown (ADR-0042).** Windows
//! stops the Job Object, including children of the Git for Windows wrapper.
//! POSIX signals the dedicated process group, which excludes descendants
//! that joined another group. Shutdown observation and pipe collection share
//! a bounded grace period. If pipes remain held after the root exits, the
//! call fails and gets one bounded cleanup window for the still-owned
//! descendants.
//!
//! **Two profiles** (DECISIONS-LOG 2026-10-20, *R1.3 — Architect decisions*):
//! - [`Git::agora`] reads no machine configuration:
//!   - no system or global config (`GIT_CONFIG_NOSYSTEM=1`,
//!     `GIT_CONFIG_GLOBAL=/dev/null`, every config-value variable dropped);
//!   - none of git's default user files either (the XDG `git/ignore` and
//!     `git/attributes`, the system attributes);
//!   - nothing the environment would choose for a new repository (a template
//!     directory, a hash, a ref format);
//!   - no attributes from the tree either (`--attr-source` at the empty tree,
//!     [`EMPTY_TREE`]), so an agent's own `.gitattributes` is committed as a
//!     file and changes nothing: one holding `working-tree-encoding` failed
//!     every `add -A`, and `text` rewrote line endings in history (measured,
//!     git 2.53; DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions,
//!     second round*). It needs git 2.40 or newer, and an older git's refusal
//!     says so.
//!
//!   It commits as `Ephesus <harness@ephesus.local>` through the identity
//!   variables, which outrank configuration from any source. So no host
//!   setting and no file in the Agora's tree can hide an Agora file from
//!   `add -A`, rewrite its line endings, run its code through a planted
//!   `core.fsmonitor`, or rename its committer. It is the committer's alone, so
//!   it is not public.
//!
//!   **`.git/` is the line.** The repository's own `.git/config` and
//!   `.git/info/attributes` are read on every call, and no git option turns the
//!   second off; either can rewrite or wedge a commit. They are outside an
//!   agent's grant, which is its own `agents/<id>/` (DECISIONS-LOG 2026-10-25,
//!   *R1.4b — Architect decisions, fourth round*, item 2); R2.3, which builds
//!   the grants and the shell, keeps an agent out of them.
//! - [`Git::target`] reads the user's configuration (`core.longpaths`,
//!   `url.insteadOf`), for worktrees and remotes of a target repository.
//!   [`Git::without_machine_config`] gives it the Agora's isolation, which is
//!   how every test runs git (the R1 exit criterion).

#[cfg(unix)]
use eph_process::unix as native;
#[cfg(windows)]
use eph_process::windows as native;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;
#[cfg(all(test, windows))]
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// The program started. A same-file constant, so the invariants check sees
/// that this crate starts git.
const PROGRAM: &str = "git";

/// The Agora's committer name.
pub const AGORA_NAME: &str = "Ephesus";

/// The Agora's committer email.
pub const AGORA_EMAIL: &str = "harness@ephesus.local";

/// How long one git call may run before it is killed (today's figure).
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// The longest deadline a caller may set: one hour, so a deadline is always a
/// time `Instant` can hold.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// How often a running git is checked against its deadline.
const POLL: Duration = Duration::from_millis(2);

/// How long output is waited for past the deadline, or after a kill: a
/// grandchild holding the pipes open is not worth a hang.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// The configuration every call carries on its command line.
const OVERRIDES: [&str; 8] = [
    "-c",
    "commit.gpgsign=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "maintenance.autoDetach=false",
    "-c",
    "gc.autoDetach=false",
];

/// The variables every call sets.
const ALWAYS: [(&str, &str); 3] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_LITERAL_PATHSPECS", "1"),
];

/// The variables that point git at a repository, dropped from every call.
/// `GIT_QUARANTINE_PATH` is set inside a `pre-receive` hook, where it forbids
/// every ref update.
const REPOSITORY_VARS: [&str; 9] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_QUARANTINE_PATH",
];

/// The machine's variables dropped when no machine configuration is read: the
/// config-value variables, and what the environment would choose for a new
/// repository or read attributes from. `GIT_CONFIG_KEY_<n>` and
/// `GIT_CONFIG_VALUE_<n>` are read only through `GIT_CONFIG_COUNT`, so
/// dropping it drops them (mutation round 2 found dropping them as well
/// changed nothing).
const MACHINE_VARS: [&str; 8] = [
    "GIT_CONFIG",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_TEMPLATE_DIR",
    "GIT_DEFAULT_HASH",
    "GIT_DEFAULT_REF_FORMAT",
    "GIT_ATTR_SOURCE",
];

/// The variables that read no machine configuration.
const NO_MACHINE_CONFIG: [(&str, &str); 3] = [
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_ATTR_NOSYSTEM", "1"),
];

/// The command-line configuration that reads no machine configuration: with
/// the keys unset, git falls back to the user's XDG `git/ignore` and
/// `git/attributes` (measured, 2026-10-20). `core.fsmonitor` would run a
/// command on every `status`.
const NO_MACHINE_OVERRIDES: [&str; 6] = [
    "-c",
    "core.excludesFile=/dev/null",
    "-c",
    "core.attributesFile=/dev/null",
    "-c",
    "core.fsmonitor=false",
];

/// git's empty tree. In a SHA-1 repository git knows it without storing it, so
/// attributes read from it are none; a SHA-256 repository refuses it on every
/// call ("bad --attr-source"), loudly (measured, git 2.53).
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// What a git older than 2.40 prints for `--attr-source`: git's own wording for
/// an option it does not know (2.53's, measured with `--not-an-option`). No
/// git that old was at hand to print it for this option itself.
const OLD_GIT: &str = "unknown option: --attr-source";

/// The sentence a refusal of `--attr-source` gains, so it names the fix.
const OLD_GIT_FIX: &str = "the Agora's git reads no attributes from the tree (--attr-source), which needs git 2.40 or newer: install a newer git";

/// The Agora's identity, as the variables that outrank any configuration.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", AGORA_NAME),
    ("GIT_AUTHOR_EMAIL", AGORA_EMAIL),
    ("GIT_COMMITTER_NAME", AGORA_NAME),
    ("GIT_COMMITTER_EMAIL", AGORA_EMAIL),
];

/// Dates a caller's environment could stamp on the Agora's commits.
const IDENTITY_DATES: [&str; 2] = ["GIT_AUTHOR_DATE", "GIT_COMMITTER_DATE"];

/// What one git call did.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitResult {
    /// What git printed on stdout, decoded lossily.
    pub stdout: String,
    /// What git printed on stderr, decoded lossily, with this runner's own
    /// sentence appended when it could not start git, stopped it, could not
    /// read its status, or never received its output.
    pub stderr: String,
    /// The exit code, or `None` in each of those cases, or when a signal ended
    /// git.
    pub code: Option<i32>,
}

impl GitResult {
    /// Whether git ran and exited 0.
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// A git runner: a profile, a deadline, and the environment it inherits.
#[derive(Debug, Clone)]
pub struct Git {
    machine_config: bool,
    tree_attributes: bool,
    identity: bool,
    timeout: Duration,
    environment: Option<Vec<(OsString, OsString)>>,
}

impl Git {
    /// The Agora's runner: no machine configuration, no attributes from the
    /// tree, the Agora's identity. The committer's alone: any other git on the
    /// Agora would bypass its queue.
    pub(crate) fn agora() -> Git {
        Git {
            machine_config: false,
            tree_attributes: false,
            identity: true,
            timeout: TIMEOUT,
            environment: None,
        }
    }

    /// A target repository's runner: the user's configuration and the
    /// repository's own attributes, no identity.
    pub fn target() -> Git {
        Git {
            machine_config: true,
            tree_attributes: true,
            identity: false,
            timeout: TIMEOUT,
            environment: None,
        }
    }

    /// The same runner reading no system or global configuration.
    pub fn without_machine_config(mut self) -> Git {
        self.machine_config = false;
        self
    }

    /// Whether this runner reads the machine's system and global configuration.
    pub fn reads_machine_config(&self) -> bool {
        self.machine_config
    }

    /// The same runner with another deadline, at most [`MAX_TIMEOUT`].
    pub fn with_timeout(mut self, timeout: Duration) -> Git {
        self.timeout = timeout.min(MAX_TIMEOUT);
        self
    }

    /// The same runner starting git with exactly `vars` as the environment it
    /// inherits, instead of this process's: how a test plays a hostile host.
    pub fn with_environment(mut self, vars: impl IntoIterator<Item = (OsString, OsString)>) -> Git {
        self.environment = Some(vars.into_iter().collect());
        self
    }

    /// Runs git in `cwd`. See the module contract.
    pub fn run(&self, cwd: &Path, args: &[&str]) -> GitResult {
        let native::Piped {
            child,
            stdout,
            stderr,
        } = match self.launch(cwd, args) {
            Ok(piped) => piped,
            Err(error) => {
                return GitResult {
                    stdout: String::new(),
                    stderr: format!("git could not be started in {}: {error}", cwd.display()),
                    code: None,
                };
            }
        };
        let stdout = drain(stdout);
        let stderr = drain(stderr);
        #[cfg(unix)]
        return complete_and_reap(child, &stdout, &stderr, self.timeout, native::Child::reap);
        #[cfg(windows)]
        {
            let mut child = child;
            complete(&mut child, &stdout, &stderr, self.timeout)
        }
    }

    #[cfg(unix)]
    fn launch(&self, cwd: &Path, args: &[&str]) -> io::Result<native::Piped> {
        native::spawn_piped(&mut self.command(cwd, args))
    }

    #[cfg(windows)]
    fn launch(&self, cwd: &Path, args: &[&str]) -> io::Result<native::Piped> {
        let command = self.command(cwd, args);
        // After env_clear, Command contains the complete supplied environment.
        // In that mode env_remove deletes an entry rather than leaving a
        // tombstone, so starting from the original values would restore it.
        let mut environment: Vec<_> = if self.environment.is_some() {
            Vec::new()
        } else {
            std::env::vars_os()
                .filter(|(name, _)| !self.drops(name))
                .collect()
        };
        for (key, value) in command.get_envs() {
            environment.retain(|(existing, _)| !existing.eq_ignore_ascii_case(key));
            if let Some(value) = value {
                environment.push((key.to_owned(), value.to_owned()));
            }
        }

        // Resolve the same program the old Command launch would find. Only an
        // explicitly supplied child PATH precedes the system/parent search.
        let child_path = self.environment.as_ref().and_then(|_| {
            environment
                .iter()
                .rev()
                .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
                .map(|(_, value)| value.as_os_str())
        });
        let executable =
            native::resolve_process_executable(std::ffi::OsStr::new("git.exe"), child_path)?;
        let cwd = std::path::absolute(cwd)?;
        let args = command.get_args().map(OsString::from).collect::<Vec<_>>();
        native::spawn_piped(native::Launch {
            executable: &executable,
            args: &args,
            cwd: &cwd,
            environment: &environment,
        })
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "git: the one place a git process is started (invariant 4, ADR-0004)"
    )]
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(PROGRAM);
        let inherited: Vec<OsString> = match &self.environment {
            Some(vars) => {
                command.env_clear();
                command.envs(vars.iter().map(|(name, value)| (name, value)));
                vars.iter().map(|(name, _)| name.clone()).collect()
            }
            None => std::env::vars_os().map(|(name, _)| name).collect(),
        };
        for name in inherited.iter().filter(|name| self.drops(name)) {
            command.env_remove(name);
        }
        command.envs(ALWAYS).args(OVERRIDES);
        if !self.machine_config {
            command.envs(NO_MACHINE_CONFIG).args(NO_MACHINE_OVERRIDES);
        }
        if !self.tree_attributes {
            command.arg(format!("--attr-source={EMPTY_TREE}"));
        }
        if self.identity {
            command.envs(IDENTITY);
        }
        command.args(args).current_dir(cwd);
        command
    }

    /// Whether an inherited variable is dropped. Names compare in upper case,
    /// as Windows compares them.
    fn drops(&self, name: &OsString) -> bool {
        let name = name.to_string_lossy().to_ascii_uppercase();
        let name = name.as_str();
        REPOSITORY_VARS.contains(&name)
            || (!self.machine_config && MACHINE_VARS.contains(&name))
            || (self.identity && IDENTITY_DATES.contains(&name))
    }
}

/// Adds [`OLD_GIT_FIX`] to a git's `stderr` that refused `--attr-source`, so
/// the refusal says what to install, not only what git did not know.
fn teach_old_git(stderr: &mut String) {
    if stderr.contains(OLD_GIT) {
        stderr.push('\n');
        stderr.push_str(OLD_GIT_FIX);
    }
}

// A root still pending after the bounded call is collected by a detached
// worker. Own the result and cleanup in one path, including failed status
// observations, so neither the wait nor its error can extend the deadline.
#[cfg(unix)]
fn complete_and_reap<C: OwnedChild + Send + 'static>(
    mut child: C,
    stdout: &Receiver<io::Result<Vec<u8>>>,
    stderr: &Receiver<io::Result<Vec<u8>>>,
    timeout: Duration,
    reap: impl FnOnce(C) + Send + 'static,
) -> GitResult {
    let result = complete(&mut child, stdout, stderr, timeout);
    if child.root_status().ok().flatten().is_none() {
        let _cleanup = thread::Builder::new()
            .name("eph-git-reap".to_owned())
            .spawn(move || reap(child));
    }
    result
}

fn complete(
    child: &mut dyn OwnedChild,
    stdout: &Receiver<io::Result<Vec<u8>>>,
    stderr: &Receiver<io::Result<Vec<u8>>>,
    timeout: Duration,
) -> GitResult {
    let deadline = Instant::now() + timeout;
    let until = drain_until(deadline);
    let exit = finish(child, deadline, until);
    let (stdout, stdout_arrived) = collect(stdout, until);
    let (mut stderr, stderr_arrived) = collect(stderr, until);
    let mut code = match &exit {
        Exit::Exited if stdout_arrived && stderr_arrived => match child.root_status() {
            Ok(Some(code)) => code,
            Ok(None) => {
                stderr.push_str("\ngit's confirmed exit status was unavailable");
                None
            }
            Err(error) => {
                stderr.push_str(&format!("\ngit's status could not be read: {error}"));
                None
            }
        },
        Exit::Exited => None,
        Exit::Stopped => {
            stderr.push_str(&format!(
                "\ngit was stopped at its {} ms deadline",
                timeout.as_millis()
            ));
            #[cfg(unix)]
            stderr
                .push_str(" (dedicated process group; any descendants in other groups may remain)");
            None
        }
        Exit::StopUnconfirmed(error) => {
            stderr.push_str(&format!(
                "\ngit's deadline shutdown could not be confirmed: {error}"
            ));
            None
        }
        Exit::Lost(error) => {
            stderr.push_str(&format!("\ngit's status could not be read: {error}"));
            None
        }
    };
    if !(stdout_arrived && stderr_arrived) {
        if matches!(exit, Exit::Exited) {
            // On POSIX the root remains unreaped, so even with a dead
            // root the group id cannot have been recycled during drain.
            if let Err(error) = stop_owned(child, Instant::now() + DRAIN_GRACE) {
                stderr.push_str(&format!(
                    "\ngit's remaining owned processes could not be stopped: {error}"
                ));
            }
        }
        stderr.push_str("\ngit's output did not arrive in time");
        code = None;
    }
    teach_old_git(&mut stderr);
    GitResult {
        stdout,
        stderr,
        code,
    }
}

/// How a git ended.
enum Exit {
    /// The root exited. Its status can be collected once pipes are drained.
    Exited,
    /// It overran its deadline and was killed.
    Stopped,
    /// Owned shutdown failed or could not be observed in time.
    StopUnconfirmed(io::Error),
    /// Its status could not be read. It is not killed: its pid may already
    /// belong to another process.
    Lost(std::io::Error),
}

/// Waits for `child` until `deadline`, then kills it.
fn finish(child: &mut dyn OwnedChild, deadline: Instant, until: Instant) -> Exit {
    loop {
        match child.root_exited() {
            Ok(true) => return Exit::Exited,
            Ok(false) => {}
            Err(error) => return Exit::Lost(error),
        }
        match deadline.checked_duration_since(Instant::now()) {
            Some(left) => thread::sleep(left.min(POLL)),
            None => {
                return match stop_owned(child, until) {
                    Ok(()) => Exit::Stopped,
                    Err(error) => Exit::StopUnconfirmed(error),
                };
            }
        }
    }
}

// These observations and the shutdown request form the OS seam. The
// runner uses the same deadline and reporting logic even on a failed syscall.
trait OwnedChild {
    fn root_exited(&mut self) -> io::Result<bool>;
    fn root_status(&mut self) -> io::Result<Option<Option<i32>>>;
    fn request_shutdown(&mut self) -> io::Result<()>;
    fn shutdown_confirmed(&mut self) -> io::Result<bool>;
}

impl OwnedChild for native::Child {
    fn root_exited(&mut self) -> io::Result<bool> {
        #[cfg(windows)]
        return self.try_wait().map(|status| status.is_some());
        #[cfg(unix)]
        self.exit_pending()
    }

    fn root_status(&mut self) -> io::Result<Option<Option<i32>>> {
        #[cfg(windows)]
        return self
            .try_wait()
            .map(|status| status.map(|code| Some(code as i32)));
        #[cfg(unix)]
        self.try_wait().map(|status| {
            status.map(|status| match status {
                native::ExitStatus::Code(code) => Some(code),
                native::ExitStatus::Signal(_) => None,
            })
        })
    }

    fn request_shutdown(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        return self.terminate();
        #[cfg(unix)]
        self.terminate_group()
    }

    fn shutdown_confirmed(&mut self) -> io::Result<bool> {
        // Job counts can reach zero before the process handle signals. Wait
        // for both before saying that the Windows job stopped.
        let root_exited = self.root_status()?.is_some();
        #[cfg(windows)]
        return Ok(root_exited && self.active_processes()? == 0);
        #[cfg(unix)]
        Ok(root_exited)
    }
}

fn stop_owned(child: &mut dyn OwnedChild, until: Instant) -> io::Result<()> {
    child.request_shutdown()?;
    loop {
        if child.shutdown_confirmed()? {
            return Ok(());
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process exit was not observed in the shutdown grace period",
            ));
        }
        thread::sleep(left.min(POLL));
    }
}

/// Reads a pipe to its end on its own thread.
fn drain<R: Read + Send + 'static>(mut pipe: R) -> Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    // A thread that cannot start drops the pipe and the sender, which reads as
    // no output: git then sees a closed pipe, not a full one.
    let _started = thread::Builder::new()
        .name("eph-git-output".to_string())
        .spawn(move || {
            let mut bytes = Vec::new();
            let outcome = pipe.read_to_end(&mut bytes).map(|_| bytes);
            let _sent = sender.send(outcome);
        });
    receiver
}

/// When waiting for output stops: the deadline, or now if it has passed, and
/// [`DRAIN_GRACE`] more.
fn drain_until(deadline: Instant) -> Instant {
    Instant::now().max(deadline) + DRAIN_GRACE
}

/// A drained pipe's text, waited for until `until`, and whether it arrived.
fn collect(output: &Receiver<io::Result<Vec<u8>>>, until: Instant) -> (String, bool) {
    let bytes = output
        .recv_timeout(until.saturating_duration_since(Instant::now()))
        .ok()
        .and_then(Result::ok);
    let arrived = bytes.is_some();
    (
        String::from_utf8_lossy(&bytes.unwrap_or_default()).into_owned(),
        arrived,
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::path::PathBuf;

    use eph_fs::atomic::write_atomic;

    use super::*;

    /// A fresh directory under the system's temporary directory.
    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eph-git-{name}-{}", std::process::id()));
        let _fresh = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The runner every test uses for a target repository: no machine
    /// configuration, so a host's git config cannot change a verdict.
    pub(crate) fn git() -> Git {
        Git::target().without_machine_config()
    }

    /// Writes `text` to `path`, making its directory.
    pub(crate) fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(path, text.as_bytes()).unwrap();
    }

    /// A link at `link` to the directory `to`: a symlink.
    #[cfg(unix)]
    pub(crate) fn link_dir(to: &Path, link: &Path) {
        std::os::unix::fs::symlink(to, link).unwrap();
    }

    /// A link at `link` to the directory `to`: a junction, which Windows makes
    /// without privilege (ADR-0021's case).
    #[cfg(windows)]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: Windows makes a junction only through `mklink /J`; std has no call for it"
    )]
    pub(crate) fn link_dir(to: &Path, link: &Path) {
        let made = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(to)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
    }

    /// Runs `args` in `cwd` and insists it succeeded.
    pub(crate) fn must(cwd: &Path, args: &[&str]) -> String {
        let result = git().run(cwd, args);
        assert!(result.ok(), "git {args:?} in {}: {result:?}", cwd.display());
        result.stdout
    }

    /// A repository at `dir` on `main` with one commit holding `README.md`.
    pub(crate) fn repo_with_commit(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        must(dir, &["init", "-b", "main"]);
        write_atomic(&dir.join("README.md"), b"# a target\n").unwrap();
        must(dir, &["add", "-A"]);
        must(
            dir,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "init",
            ],
        );
    }

    /// This process's environment without any `GIT_` variable, plus `extra`:
    /// a host the test controls completely.
    pub(crate) fn host_with(extra: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| {
                !name
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("GIT_")
            })
            .collect();
        vars.extend(
            extra
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value))),
        );
        vars
    }

    /// A global config file naming Mallory and ignoring every `.json` file.
    fn hostile_config(dir: &Path) -> PathBuf {
        let ignore = dir.join("hostile.ignore");
        write_atomic(&ignore, b"*.json\n").unwrap();
        let path = dir.join("hostile.gitconfig");
        let text = format!(
            "[user]\n\tname = Mallory\n\temail = mallory@example.invalid\n[core]\n\texcludesFile = {}\n",
            ignore.display().to_string().replace('\\', "/")
        );
        write_atomic(&path, text.as_bytes()).unwrap();
        path
    }

    /// Sets `key` in `repo`'s own config, which every profile reads.
    fn set_in_repo(repo: &Path, key: &str, value: &str) {
        must(repo, &["config", "--file", ".git/config", key, value]);
    }

    #[test]
    fn a_result_is_ok_only_for_exit_code_zero() {
        let result = |code| GitResult {
            stdout: String::new(),
            stderr: String::new(),
            code,
        };
        assert!(result(Some(0)).ok());
        assert!(!result(Some(1)).ok());
        assert!(!result(None).ok());
    }

    #[test]
    fn process_observation_failures_preserve_output_and_cannot_report_success() {
        struct Child {
            observed: io::Result<bool>,
            status: io::Result<Option<Option<i32>>>,
            shutdown_fails: bool,
            confirmations: std::vec::IntoIter<bool>,
        }

        impl OwnedChild for Child {
            fn root_exited(&mut self) -> io::Result<bool> {
                std::mem::replace(&mut self.observed, Ok(false))
            }

            fn root_status(&mut self) -> io::Result<Option<Option<i32>>> {
                std::mem::replace(&mut self.status, Ok(None))
            }

            fn request_shutdown(&mut self) -> io::Result<()> {
                if self.shutdown_fails {
                    Err(io::Error::other("shutdown error"))
                } else {
                    Ok(())
                }
            }

            fn shutdown_confirmed(&mut self) -> io::Result<bool> {
                Ok(self.confirmations.next().unwrap_or(false))
            }
        }

        fn pipe(bytes: Option<&[u8]>) -> Receiver<io::Result<Vec<u8>>> {
            let (send, receive) = mpsc::channel();
            if let Some(bytes) = bytes {
                send.send(Ok(bytes.to_vec())).unwrap();
            }
            receive
        }

        for (observed, status, shutdown_fails, output, diagnostic) in [
            (
                Ok(true),
                Ok(None),
                false,
                Some(b"output".as_slice()),
                "confirmed exit status was unavailable",
            ),
            (
                Ok(true),
                Err(io::Error::other("status error")),
                false,
                Some(b"output"),
                "status could not be read: status error",
            ),
            (
                Err(io::Error::other("wait error")),
                Ok(None),
                false,
                Some(b"output"),
                "status could not be read: wait error",
            ),
            (
                Ok(false),
                Ok(None),
                true,
                Some(b"output"),
                "deadline shutdown could not be confirmed: shutdown error",
            ),
            (
                Ok(true),
                Ok(None),
                true,
                None,
                "remaining owned processes could not be stopped: shutdown error",
            ),
        ] {
            let mut child = Child {
                observed,
                status,
                shutdown_fails,
                confirmations: Vec::new().into_iter(),
            };
            let result = complete(
                &mut child,
                &pipe(output),
                &pipe(Some(b"child diagnostic")),
                Duration::ZERO,
            );
            assert_eq!(result.code, None);
            assert!(result.stderr.starts_with("child diagnostic\n"));
            assert!(result.stderr.contains(diagnostic), "{result:?}");
            assert_eq!(result.stdout.as_bytes(), output.unwrap_or_default());
        }
        let mut still_running = Child {
            observed: Ok(false),
            status: Ok(None),
            shutdown_fails: false,
            confirmations: Vec::new().into_iter(),
        };
        let error = stop_owned(&mut still_running, Instant::now()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);

        let mut confirming = Child {
            confirmations: vec![false, true].into_iter(),
            ..still_running
        };
        stop_owned(&mut confirming, Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(confirming.confirmations.len(), 0);
    }

    #[test]
    fn missing_either_pipe_keeps_the_root_owned_until_cleanup_is_confirmed() {
        struct Child {
            reads: usize,
            requests: usize,
            confirmations: std::vec::IntoIter<bool>,
        }
        impl OwnedChild for Child {
            fn root_exited(&mut self) -> io::Result<bool> {
                Ok(true)
            }
            fn root_status(&mut self) -> io::Result<Option<Option<i32>>> {
                self.reads += 1;
                Ok(Some(Some(0)))
            }
            fn request_shutdown(&mut self) -> io::Result<()> {
                self.requests += 1;
                Ok(())
            }
            fn shutdown_confirmed(&mut self) -> io::Result<bool> {
                Ok(self.confirmations.next().unwrap())
            }
        }
        fn pipe(arrived: bool) -> Receiver<io::Result<Vec<u8>>> {
            let (sent, received) = mpsc::channel();
            if arrived {
                sent.send(Ok(b"preserved output".to_vec())).unwrap();
            }
            received
        }
        for (stdout, stderr) in [(false, true), (true, false)] {
            let mut child = Child {
                reads: 0,
                requests: 0,
                confirmations: vec![false, true].into_iter(),
            };
            let result = complete(&mut child, &pipe(stdout), &pipe(stderr), Duration::ZERO);
            assert_eq!(result.code, None);
            assert!(
                result
                    .stderr
                    .ends_with("git's output did not arrive in time")
            );
            assert!(
                !result.stderr.contains("could not be stopped"),
                "{result:?}"
            );
            assert_eq!(
                child.reads, 0,
                "the root must remain unreaped while pipes are held"
            );
            assert_eq!(child.requests, 1);
            assert_eq!(
                child.confirmations.len(),
                0,
                "cleanup received its own grace interval"
            );
        }
    }

    #[test]
    fn native_adapter_observes_a_ready_child_before_and_after_owned_shutdown() {
        use std::io::BufRead as _;

        fn actually_exited(child: &mut native::Child) -> bool {
            #[cfg(windows)]
            return child.try_wait().unwrap().is_some();
            #[cfg(unix)]
            child.exit_pending().unwrap()
        }
        fn actually_stopped(child: &mut native::Child) -> bool {
            let exited = actually_exited(child);
            #[cfg(windows)]
            return exited && child.active_processes().unwrap() == 0;
            #[cfg(unix)]
            exited
        }
        let live_alias = if cfg!(target_os = "macos") {
            // Record Git's PID and group before the shell starts sleep.
            r#"alias.held=!group=$(/bin/ps -o pgid= -p "$PPID"); printf 'native-ready %s %s\n' "$PPID" "$group"; sleep 10"#
        } else {
            "alias.held=!echo native-ready; sleep 10"
        };
        let mut cases = vec![(false, live_alias)];
        #[cfg(windows)]
        cases.push((
            true,
            "alias.held=!(echo native-ready; sleep 10) & echo root-exiting",
        ));
        for (root_exited, alias) in cases.drain(..) {
            let dir = scratch(if root_exited {
                "adapter-exited-root"
            } else {
                "adapter-live-root"
            });
            let native::Piped {
                mut child,
                stdout,
                stderr,
            } = git().launch(&dir, &["-c", alias, "held"]).unwrap();
            let mut output = std::io::BufReader::new(stdout);
            let mut line = String::new();
            while !line.contains("native-ready") {
                line.clear();
                assert_ne!(output.read_line(&mut line).unwrap(), 0);
            }
            #[cfg(target_os = "macos")]
            let group = {
                let mut ids = line
                    .strip_prefix("native-ready ")
                    .expect("the fixture reports its native process group")
                    .split_whitespace()
                    .map(str::parse::<u32>);
                let root = ids.next().unwrap().unwrap();
                let group = ids.next().unwrap().unwrap();
                assert!(
                    root > 1 && root == group,
                    "unexpected fixture identity: {line}"
                );
                group
            };
            let deadline = Instant::now() + Duration::from_secs(3);
            while actually_exited(&mut child) != root_exited {
                assert!(
                    Instant::now() < deadline,
                    "fixture did not reach its expected root state"
                );
                thread::sleep(POLL);
            }
            #[cfg(windows)]
            let expected_status = {
                assert!(child.active_processes().unwrap() > 0);
                if root_exited {
                    Some(Some(child.try_wait().unwrap().unwrap() as i32))
                } else {
                    Some(Some(1))
                }
            };
            let observed_exit = child.root_exited().unwrap();
            let observed_shutdown = child.shutdown_confirmed().unwrap();
            #[cfg(windows)]
            child.terminate().unwrap();
            #[cfg(unix)]
            child.terminate_group().unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !actually_stopped(&mut child) {
                assert!(
                    Instant::now() < deadline,
                    "native cleanup was not confirmed"
                );
                thread::sleep(POLL);
            }
            #[cfg(target_os = "macos")]
            for (stream, ended) in [("stdout", drain(output)), ("stderr", drain(stderr))] {
                match ended.recv_timeout(Duration::from_secs(3)) {
                    Ok(result) => {
                        result.unwrap();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // Keep the dead root unreaped until after examining its
                        // group and attempting cleanup of any remaining members.
                        let members = git().with_timeout(Duration::from_secs(2)).run(
                            &dir,
                            &[
                                "-c",
                                r#"alias.group-state=!f() { /bin/ps -A -o pid= -o pgid= -o state= -o ucomm= | /usr/bin/awk -v group="$1" '$2 == group'; }; f"#,
                                "group-state",
                                &group.to_string(),
                            ],
                        );
                        let repeated = child.terminate_group();
                        let eof = ended.recv_timeout(DRAIN_GRACE).map(|read| read.map(|_| ()));
                        let root = child.try_wait();
                        panic!(
                            "{stream} stayed open after owned shutdown; group {group} members \
                             before a second signal: {members:?}; second signal: {repeated:?}; \
                             pipe after second signal: {eof:?}; root: {root:?}"
                        );
                    }
                    Err(error) => panic!("{stream} drain worker disconnected: {error}"),
                }
            }
            assert!(child.try_wait().unwrap().is_some());
            let final_status = child.root_status().unwrap();
            let final_exit = child.root_exited().unwrap();
            let final_shutdown = child.shutdown_confirmed().unwrap();
            #[cfg(not(target_os = "macos"))]
            for ended in [drain(output), drain(stderr)] {
                ended.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
            }
            drop(child);
            fs::remove_dir_all(dir).unwrap();
            // Cleanup precedes the assertions even for a lying adapter mutant.
            assert_eq!(observed_exit, root_exited);
            assert!(!observed_shutdown, "an owned member is still alive");
            #[cfg(windows)]
            assert_eq!(final_status, expected_status);
            #[cfg(unix)]
            assert_eq!(final_status, Some(None));
            assert!(final_exit && final_shutdown);
        }
    }

    #[cfg(windows)]
    #[test]
    fn inherited_environment_fixture() {
        let Ok(expected) = std::env::var("EPH_TEST_INHERITED_PLAIN") else {
            return;
        };
        let dir = scratch("inherited-environment-child");
        let result = git().run(
            &dir,
            &[
                "-c",
                "alias.show-parent=!printf '%s' \"$EPH_TEST_INHERITED_PLAIN\"",
                "show-parent",
            ],
        );
        assert!(result.ok(), "{result:?}");
        assert_eq!(result.stdout, expected);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: supply a controlled parent environment in a disposable test process"
    )]
    fn an_ordinary_inherited_variable_reaches_native_windows_launch() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runner::tests::inherited_environment_fixture",
                "--nocapture",
            ])
            .env("EPH_TEST_INHERITED_PLAIN", "preserved-parent-value")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "the inherited launch lost the supplied ordinary value"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pending_or_failed_final_status_is_reaped_without_blocking_the_result() {
        struct Child(Option<io::Result<Option<Option<i32>>>>);
        impl OwnedChild for Child {
            fn root_exited(&mut self) -> io::Result<bool> {
                Err(io::Error::other("observation failed"))
            }
            fn root_status(&mut self) -> io::Result<Option<Option<i32>>> {
                self.0.take().unwrap()
            }
            fn request_shutdown(&mut self) -> io::Result<()> {
                panic!("a failed observation cannot send a signal")
            }
            fn shutdown_confirmed(&mut self) -> io::Result<bool> {
                panic!("no shutdown was requested")
            }
        }
        fn pipe(bytes: &[u8]) -> Receiver<io::Result<Vec<u8>>> {
            let (sent, received) = mpsc::channel();
            sent.send(Ok(bytes.to_vec())).unwrap();
            received
        }
        for (status, expected_reap) in [
            (Ok(None), true),
            (Err(io::Error::other("final status failed")), true),
            (Ok(Some(Some(0))), false),
            (Ok(Some(None)), false),
        ] {
            let (started, observed_start) = mpsc::channel();
            let (release, gate) = mpsc::channel();
            let (finished, observed_finish) = mpsc::channel();
            let (sent, received) = mpsc::channel();
            let caller = thread::spawn(move || {
                let result = complete_and_reap(
                    Child(Some(status)),
                    &pipe(b"preserved output"),
                    &pipe(b"preserved diagnostic"),
                    Duration::ZERO,
                    move |_| {
                        started.send(()).unwrap();
                        gate.recv().unwrap();
                        finished.send(()).unwrap();
                    },
                );
                sent.send(result).unwrap();
            });
            let result = received.recv_timeout(Duration::from_secs(2));
            let began = observed_start.recv_timeout(Duration::from_secs(2));
            // Release even when an aimed synchronous-reap mutation blocks the
            // caller, then join before asserting: a timeout is not a kill.
            let _released = release.send(());
            caller.join().unwrap();
            assert!(result.is_ok(), "the result waited for the reaper");
            let result = result.unwrap();
            assert_eq!(result.stdout, "preserved output");
            assert!(result.stderr.starts_with("preserved diagnostic\n"));
            assert!(result.stderr.contains("observation failed"));
            assert_eq!(result.code, None);
            if expected_reap {
                began.unwrap();
                observed_finish
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
            } else {
                assert!(matches!(began, Err(mpsc::RecvTimeoutError::Disconnected)));
                assert!(matches!(
                    observed_finish.try_recv(),
                    Err(mpsc::TryRecvError::Disconnected)
                ));
            }
        }
    }

    #[test]
    fn the_agora_profile_reads_no_machine_configuration() {
        let dir = scratch("agora-config");
        let hostile = hostile_config(&dir);
        let host = host_with(&[
            ("GIT_CONFIG_GLOBAL", hostile.to_str().unwrap()),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "alias.planted"),
            ("GIT_CONFIG_VALUE_0", "status"),
            ("GIT_CONFIG_PARAMETERS", "'alias.smuggled'='status'"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]);
        let listed = |git: Git| {
            git.with_environment(host.clone())
                .run(&dir, &["config", "--list"])
        };
        let agora = listed(Git::agora());
        assert!(agora.ok(), "{agora:?}");
        assert!(!agora.stdout.contains("Mallory"), "{}", agora.stdout);
        assert!(!agora.stdout.contains("alias.planted"), "{}", agora.stdout);
        assert!(!agora.stdout.contains("alias.smuggled"), "{}", agora.stdout);
        assert!(
            agora.stdout.contains("commit.gpgsign=false"),
            "{}",
            agora.stdout
        );
        // The same host DOES reach a runner that reads it, so the plants are live.
        let target = listed(Git::target());
        assert!(
            target.stdout.contains("user.name=Mallory"),
            "{}",
            target.stdout
        );
        assert!(
            target.stdout.contains("alias.planted=status"),
            "{}",
            target.stdout
        );
        assert!(
            target.stdout.contains("alias.smuggled=status"),
            "{}",
            target.stdout
        );
        assert!(Git::target().reads_machine_config());
        assert!(!Git::agora().reads_machine_config());
        assert!(
            !Git::target()
                .without_machine_config()
                .reads_machine_config()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_agora_commits_as_itself_whatever_the_host_exports() {
        let dir = scratch("identity");
        let hostile = hostile_config(&dir);
        let repo = dir.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let host = host_with(&[
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", hostile.to_str().unwrap()),
            ("GIT_AUTHOR_NAME", "Mallory"),
            ("GIT_AUTHOR_EMAIL", "mallory@example.invalid"),
            ("GIT_COMMITTER_NAME", "Mallory"),
            ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
        ]);
        let agora = Git::agora().with_environment(host.clone());
        assert!(agora.run(&repo, &["init", "-b", "main"]).ok());
        // The repository's own config asks for a signature no gpg can give.
        set_in_repo(&repo, "commit.gpgsign", "true");
        set_in_repo(&repo, "gpg.program", "eph-no-such-gpg");
        write_atomic(&repo.join("registry.json"), b"{}\n").unwrap();
        let status = |git: &Git| git.run(&repo, &["status", "--porcelain"]).stdout;
        assert!(
            !status(&Git::target().with_environment(host.clone())).contains("registry.json"),
            "the plant is live: a runner reading the host's config is blind to the file"
        );
        assert!(
            status(&agora).contains("registry.json"),
            "a hostile excludesFile cannot hide an Agora file"
        );
        assert!(agora.run(&repo, &["add", "-A"]).ok());
        let committed = agora.run(&repo, &["commit", "-m", "one"]);
        assert!(
            committed.ok(),
            "the repository's gpgsign cannot stop it: {committed:?}"
        );
        let log = agora.run(&repo, &["log", "-1", "--format=%an <%ae>|%cn <%ce>|%ad"]);
        let line = log.stdout.trim();
        assert!(
            line.starts_with("Ephesus <harness@ephesus.local>|Ephesus <harness@ephesus.local>|"),
            "{line}"
        );
        assert!(
            !line.contains("2001"),
            "the host's date is not the Agora's: {line}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_call_drops_the_variables_that_point_at_a_repository() {
        let dir = scratch("repository-vars");
        let mine = dir.join("mine");
        let other = dir.join("other");
        repo_with_commit(&mine);
        repo_with_commit(&other);
        let other_git = other.join(".git");
        let host = host_with(&[
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_DIR", other_git.to_str().unwrap()),
            ("GIT_WORK_TREE", other.to_str().unwrap()),
            ("git_index_file", "nowhere"),
            ("GIT_QUARANTINE_PATH", "nowhere"),
        ]);
        for git in [Git::agora(), Git::target()] {
            let top = git
                .with_environment(host.clone())
                .run(&mine, &["rev-parse", "--absolute-git-dir"]);
            assert!(top.ok(), "{top:?}");
            assert_eq!(
                fs::canonicalize(top.stdout.trim()).unwrap(),
                fs::canonicalize(mine.join(".git")).unwrap(),
                "git ran against the directory it was given"
            );
        }
        // Inside a pre-receive hook git forbids every ref update; the variable
        // that says so is dropped with the others.
        let committed = git().with_environment(host).run(
            &mine,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "q",
            ],
        );
        assert!(committed.ok(), "{committed:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_call_asks_nothing_takes_no_optional_lock_and_reads_pathspecs_literally() {
        let dir = scratch("always");
        let shown = git().run(
            &dir,
            &[
                "-c",
                "alias.eph-env=!echo \"$GIT_TERMINAL_PROMPT $GIT_OPTIONAL_LOCKS $GIT_LITERAL_PATHSPECS\"",
                "eph-env",
            ],
        );
        assert!(shown.ok(), "{shown:?}");
        assert_eq!(shown.stdout.trim(), "0 0 1");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_command_line_overrides_win_over_the_repositorys_own_config() {
        let dir = scratch("overrides");
        must(&dir, &["init", "-b", "main"]);
        let planted = [
            ("commit.gpgsign", "true", "false"),
            ("core.hooksPath", "elsewhere", "/dev/null"),
            ("maintenance.autoDetach", "true", "false"),
            ("gc.autoDetach", "true", "false"),
        ];
        for (key, value, _) in planted {
            set_in_repo(&dir, key, value);
        }
        for git in [Git::agora(), git()] {
            for (key, value, wins) in planted {
                let read = |args: &[&str]| git.run(&dir, args).stdout.trim().to_string();
                assert_eq!(
                    read(&["config", "--file", ".git/config", "--get", key]),
                    value
                );
                assert_eq!(read(&["config", "--get", key]), wins, "{key}");
            }
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_git_reports_its_code_and_its_words() {
        let dir = scratch("failed");
        let result = git().run(&dir, &["rev-parse", "--git-dir"]);
        assert!(!result.ok());
        assert_eq!(result.code, Some(128));
        assert!(result.stderr.contains("not a git repository"), "{result:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_git_that_cannot_start_is_a_result_not_a_panic() {
        let dir = scratch("cannot-start");
        let missing = dir.join("absent");
        let file = dir.join("a-file");
        write_atomic(&file, b"x").unwrap();
        for cwd in [&missing, &file] {
            let result = git().run(cwd, &["status"]);
            assert_eq!(result.code, None, "{result:?}");
            assert!(
                result.stderr.starts_with("git could not be started in "),
                "{result:?}"
            );
            assert!(
                result.stderr.contains(&cwd.display().to_string()),
                "{result:?}"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_git_that_fills_both_pipes_is_never_stalled() {
        let dir = scratch("pipes");
        let big = "0123456789abcdef".repeat(64 * 1024);
        write_atomic(&dir.join("big.txt"), big.as_bytes()).unwrap();
        let loud = git().run(
            &dir,
            &["-c", "alias.loud=!cat big.txt; cat big.txt >&2", "loud"],
        );
        assert!(loud.ok(), "{}", loud.stderr.len());
        assert_eq!(loud.stdout.len(), big.len(), "a megabyte on stdout");
        assert_eq!(loud.stderr.len(), big.len(), "and on stderr");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_git_past_its_deadline_is_stopped_and_says_so() {
        let dir = scratch("deadline");
        let started = Instant::now();
        let result = git()
            .with_timeout(Duration::from_millis(300))
            .run(&dir, &["-c", "alias.nap=!sleep 5", "nap"]);
        let took = started.elapsed();
        assert_eq!(result.code, None, "{result:?}");
        assert!(
            result
                .stderr
                .contains("\ngit was stopped at its 300 ms deadline"),
            "{result:?}"
        );
        assert!(took >= Duration::from_millis(300), "{took:?}");
        assert!(
            took < Duration::from_secs(3),
            "a grandchild holding the pipes cannot hold the call: {took:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn output_that_never_arrives_is_a_failure_not_an_empty_success() {
        let dir = scratch("output-lost");
        let started = Instant::now();
        // git exits well inside its deadline, even on a loaded machine; the
        // backgrounded sleep keeps its pipes open well past deadline and grace.
        let result = git().with_timeout(Duration::from_secs(2)).run(
            &dir,
            &["-c", "alias.linger=!(sleep 10 &); echo hi", "linger"],
        );
        assert_eq!(result.code, None, "{result:?}");
        assert!(
            result
                .stderr
                .ends_with("\ngit's output did not arrive in time"),
            "{result:?}"
        );
        assert!(
            !result.stderr.contains("stopped"),
            "git itself exited: {result:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "deadline plus grace, not the sleep"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_descendant_holding_output_past_the_deadline_cannot_continue_after_the_root_exits() {
        let dir = scratch("output-deadline-owned-descendant");
        let start = Instant::now();
        let result = git().with_timeout(Duration::from_secs(2)).run(
            &dir,
            &[
                "-c",
                "alias.linger=!(echo ready > started; sleep 4; echo survived > late) & echo root-exiting",
                "linger",
            ],
        );
        assert!(dir.join("started").is_file(), "the descendant actually ran");
        assert!(
            result
                .stderr
                .ends_with("\ngit's output did not arrive in time"),
            "{result:?}"
        );
        assert!(
            !result.stderr.contains("could not be stopped"),
            "{result:?}"
        );
        thread::sleep(Duration::from_secs(5).saturating_sub(start.elapsed()));
        assert!(
            !dir.join("late").exists(),
            "a descendant survived the deadline"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_deadline_beyond_what_an_instant_can_hold_is_clamped() {
        let dir = scratch("clamped");
        let clamped = git().with_timeout(Duration::MAX);
        assert_eq!(clamped.timeout, MAX_TIMEOUT);
        assert_eq!(MAX_TIMEOUT, Duration::from_secs(3600), "one hour");
        let result = clamped.run(&dir, &["--version"]);
        assert!(result.ok(), "{result:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn output_lost_on_either_pipe_alone_is_a_failure() {
        let dir = scratch("one-pipe");
        // Each backgrounded sleep keeps one pipe open and sends the other to
        // /dev/null, so exactly one stream never ends.
        for (holds, alias) in [
            ("stdout", "alias.linger=!(sleep 10 2>/dev/null &); echo hi"),
            ("stderr", "alias.linger=!(sleep 10 >/dev/null &); echo hi"),
        ] {
            let result = git()
                .with_timeout(Duration::from_secs(2))
                .run(&dir, &["-c", alias, "linger"]);
            assert_eq!(result.code, None, "{holds}: {result:?}");
            assert!(
                result
                    .stderr
                    .ends_with("\ngit's output did not arrive in time"),
                "{holds}: {result:?}"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_profile_drops_only_what_it_names_and_keeps_the_rest() {
        let dir = scratch("keeps");
        let host = host_with(&[
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("EPH_PLAIN", "kept"),
            ("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z"),
        ]);
        let shown = |git: Git| {
            git.with_environment(host.clone())
                .run(
                    &dir,
                    &[
                        "-c",
                        "alias.eph-env=!echo \"[$EPH_PLAIN] [$GIT_AUTHOR_DATE]\"",
                        "eph-env",
                    ],
                )
                .stdout
                .trim()
                .to_string()
        };
        assert_eq!(
            shown(Git::agora()),
            "[kept] []",
            "the Agora keeps an ordinary variable and drops the date"
        );
        assert_eq!(
            shown(git()),
            "[kept] [2001-01-01T00:00:00Z]",
            "a target keeps both: it commits nothing of the Agora's"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_machine_config_holds_for_a_host_that_never_asked_for_it() {
        let dir = scratch("nosystem");
        let host = host_with(&[]);
        let shown = |git: Git| {
            git.with_environment(host.clone())
                .run(
                    &dir,
                    &[
                        "-c",
                        "alias.eph-env=!echo \"[$GIT_CONFIG_NOSYSTEM] [$GIT_CONFIG_GLOBAL] [$GIT_ATTR_NOSYSTEM]\"",
                        "eph-env",
                    ],
                )
                .stdout
                .trim()
                .to_string()
        };
        assert_eq!(shown(Git::agora()), "[1] [/dev/null] [1]");
        assert_eq!(shown(git()), "[1] [/dev/null] [1]");
        assert_eq!(
            shown(Git::target()),
            "[] [] []",
            "the user's profile sets none of them"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_agora_reads_no_users_default_ignore_or_attributes_file() {
        let dir = scratch("xdg");
        let xdg = dir.join("xdg");
        write_atomic_in(&xdg.join("git").join("ignore"), "*.json\n");
        write_atomic_in(&xdg.join("git").join("attributes"), "*.md text eol=crlf\n");
        let empty = dir.join("empty.gitconfig");
        write_atomic(&empty, b"").unwrap();
        let host = host_with(&[
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", empty.to_str().unwrap()),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ]);
        let repo = dir.join("repo");
        repo_with_commit(&repo);
        write_atomic(&repo.join("registry.json"), b"{}\n").unwrap();
        let status = |git: Git| {
            git.with_environment(host.clone())
                .run(&repo, &["status", "--porcelain"])
                .stdout
        };
        let eol = |git: Git| {
            git.with_environment(host.clone())
                .run(&repo, &["check-attr", "eol", "--", "a.md"])
                .stdout
        };
        assert!(
            !status(Git::target()).contains("registry.json"),
            "the plant is live for the user's profile"
        );
        assert!(
            eol(Git::target()).contains("eol: crlf"),
            "the plant is live for the user's profile"
        );
        assert!(
            status(Git::agora()).contains("registry.json"),
            "an XDG ignore cannot hide an Agora file"
        );
        assert!(
            eol(Git::agora()).contains("eol: unspecified"),
            "nor an XDG attributes file rewrite one"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_environment_cannot_choose_a_new_agoras_template_or_hash() {
        let dir = scratch("template");
        let template = dir.join("template");
        write_atomic_in(&template.join("info").join("exclude"), "*.txt\n");
        let host = host_with(&[
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_TEMPLATE_DIR", template.to_str().unwrap()),
            ("GIT_DEFAULT_HASH", "sha256"),
        ]);
        for (git, name, planted) in [
            (Git::target(), "user", true),
            (Git::agora(), "agora", false),
        ] {
            let repo = dir.join(name);
            fs::create_dir_all(&repo).unwrap();
            let git = git.with_environment(host.clone());
            assert!(git.run(&repo, &["init", "-b", "main"]).ok());
            let format = git
                .run(&repo, &["rev-parse", "--show-object-format"])
                .stdout;
            assert_eq!(format.trim() == "sha256", planted, "{name}: {format}");
            let exclude = fs::read_to_string(repo.join(".git").join("info").join("exclude"))
                .unwrap_or_default();
            assert_eq!(exclude.contains("*.txt"), planted, "{name}: {exclude}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, second round*,
    /// item 2: measured on git 2.53, one agent's `.gitattributes` failed every
    /// `add -A`, and `text` rewrote line endings in history.
    #[test]
    fn the_agora_reads_no_attributes_from_the_tree() {
        let dir = scratch("tree-attributes");
        for (git, name, reads) in [(git(), "target", true), (Git::agora(), "agora", false)] {
            let repo = dir.join(name);
            fs::create_dir_all(&repo).unwrap();
            assert!(git.run(&repo, &["init", "-b", "main"]).ok(), "{name}");
            write(&repo.join(".gitattributes"), "*.txt text\n");
            write(&repo.join("crlf.txt"), "a\r\nb\r\n");
            let staged = git.run(&repo, &["add", "-A"]);
            assert!(staged.ok(), "{name}: {staged:?}");
            let blob = git.run(&repo, &["cat-file", "-p", ":crlf.txt"]).stdout;
            assert_eq!(
                blob == "a\nb\n",
                reads,
                "{name}: `text` normalises only where the tree's attributes are read: {blob:?}"
            );
            write(
                &repo.join("agents").join("a1").join(".gitattributes"),
                "* working-tree-encoding=UTF-16\n",
            );
            let staged = git.run(&repo, &["add", "-A"]);
            assert_eq!(
                staged.ok(),
                !reads,
                "{name}: an agent's encoding attribute fails `add -A` only where it is read: {staged:?}"
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_git_too_old_for_attr_source_says_which_git_to_install() {
        let dir = scratch("old-git");
        let said = |alias: &str| {
            git().run(
                &dir,
                &[
                    "-c",
                    &format!("alias.said=!echo '{alias}' >&2; exit 129"),
                    "said",
                ],
            )
        };
        let old = said("unknown option: --attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        assert_eq!(old.code, Some(129), "{old:?}");
        assert!(
            old.stderr.ends_with(OLD_GIT_FIX) && OLD_GIT_FIX.contains("git 2.40 or newer"),
            "the refusal names the fix: {old:?}"
        );
        let other = said("unknown option: --colour");
        assert!(!other.stderr.contains(OLD_GIT_FIX), "{other:?}");
        fs::remove_dir_all(&dir).unwrap();
    }

    fn write_atomic_in(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(path, text.as_bytes()).unwrap();
    }
}

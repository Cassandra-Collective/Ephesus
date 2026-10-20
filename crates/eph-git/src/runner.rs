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
//! **The deadline (20 s) bounds the call, not git.** Past it, the process this
//! runner started is killed and the call returns. That process is not always
//! the whole git: on Windows the `git.exe` on PATH is Git for Windows' wrapper,
//! and killing it leaves the real git running (measured, 2026-10-20). On POSIX
//! the kill does not reach git's own children, such as a commit's housekeeping.
//! So a git killed at its deadline may finish its work later, or leave a lock
//! behind. Killing the whole tree is R2's, with `eph-pty`'s Job Objects and
//! process groups (DECISIONS-LOG 2026-10-20, *R1.3 — Architect decisions,
//! after review*). Output is waited for until the deadline and half a second
//! more. A grandchild such as a shell alias can hold the pipes open after git
//! has exited, and output that never arrives makes the call a failure, not an
//! empty success.
//!
//! **Two profiles** (DECISIONS-LOG 2026-10-20, *R1.3 — Architect decisions*):
//! - [`Git::agora`] reads no machine configuration:
//!   - no system or global config (`GIT_CONFIG_NOSYSTEM=1`,
//!     `GIT_CONFIG_GLOBAL=/dev/null`, every config-value variable dropped);
//!   - none of git's default user files either (the XDG `git/ignore` and
//!     `git/attributes`, the system attributes);
//!   - nothing the environment would choose for a new repository (a template
//!     directory, a hash, a ref format).
//!
//!   It commits as `Ephesus <harness@ephesus.local>` through the identity
//!   variables, which outrank configuration from any source. So no host
//!   setting can hide an Agora file from `add -A`, rewrite its line endings,
//!   run its code through a planted `core.fsmonitor`, or rename its committer.
//!   It is the committer's alone, so it is not public.
//! - [`Git::target`] reads the user's configuration (`core.longpaths`,
//!   `url.insteadOf`), for worktrees and remotes of a target repository.
//!   [`Git::without_machine_config`] gives it the Agora's isolation, which is
//!   how every test runs git (the R1 exit criterion).

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
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

/// The machine's variables dropped when no machine configuration is read,
/// beside every `GIT_CONFIG_KEY_<n>` and `GIT_CONFIG_VALUE_<n>`: the
/// config-value variables, and what the environment would choose for a new
/// repository or read attributes from.
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

/// The Agora's identity, as the variables that outrank any configuration.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", AGORA_NAME),
    ("GIT_AUTHOR_EMAIL", AGORA_EMAIL),
    ("GIT_COMMITTER_NAME", AGORA_NAME),
    ("GIT_COMMITTER_EMAIL", AGORA_EMAIL),
];

/// Dates a caller's environment could stamp on the Agora's commits.
const IDENTITY_DATES: [&str; 2] = ["GIT_AUTHOR_DATE", "GIT_COMMITTER_DATE"];

/// `CREATE_NO_WINDOW`: a git started by the windowed build opens no console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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
    identity: bool,
    timeout: Duration,
    environment: Option<Vec<(OsString, OsString)>>,
}

impl Git {
    /// The Agora's runner: no machine configuration, the Agora's identity. The
    /// committer's alone: any other git on the Agora would bypass its queue.
    pub(crate) fn agora() -> Git {
        Git {
            machine_config: false,
            identity: true,
            timeout: TIMEOUT,
            environment: None,
        }
    }

    /// A target repository's runner: the user's configuration, no identity.
    pub fn target() -> Git {
        Git {
            machine_config: true,
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
        let mut child = match self.command(cwd, args).spawn() {
            Ok(child) => child,
            Err(error) => {
                return GitResult {
                    stdout: String::new(),
                    stderr: format!("git could not be started in {}: {error}", cwd.display()),
                    code: None,
                };
            }
        };
        let stdout = child.stdout.take().map(drain);
        let stderr = child.stderr.take().map(drain);
        let deadline = Instant::now() + self.timeout;
        let exit = finish(&mut child, deadline);
        let until = Instant::now().max(deadline) + DRAIN_GRACE;
        let (stdout, stdout_arrived) = collect(stdout.as_ref(), until);
        let (mut stderr, stderr_arrived) = collect(stderr.as_ref(), until);
        let mut code = match exit {
            Exit::Exited(code) => code,
            Exit::Stopped => {
                stderr.push_str(&format!(
                    "\ngit was stopped at its {} ms deadline",
                    self.timeout.as_millis()
                ));
                None
            }
            Exit::Lost(error) => {
                stderr.push_str(&format!("\ngit's status could not be read: {error}"));
                None
            }
        };
        if !(stdout_arrived && stderr_arrived) {
            stderr.push_str("\ngit's output did not arrive in time");
            code = None;
        }
        GitResult {
            stdout,
            stderr,
            code,
        }
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
        if self.identity {
            command.envs(IDENTITY);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Whether an inherited variable is dropped. Names compare in upper case,
    /// as Windows compares them.
    fn drops(&self, name: &OsString) -> bool {
        let name = name.to_string_lossy().to_ascii_uppercase();
        let name = name.as_str();
        REPOSITORY_VARS.contains(&name)
            || (!self.machine_config
                && (MACHINE_VARS.contains(&name)
                    || name.starts_with("GIT_CONFIG_KEY_")
                    || name.starts_with("GIT_CONFIG_VALUE_")))
            || (self.identity && IDENTITY_DATES.contains(&name))
    }
}

/// How a git ended.
enum Exit {
    /// It exited, with this code (`None` for a signal).
    Exited(Option<i32>),
    /// It overran its deadline and was killed.
    Stopped,
    /// Its status could not be read. It is not killed: its pid may already
    /// belong to another process.
    Lost(std::io::Error),
}

/// Waits for `child` until `deadline`, then kills it.
fn finish(child: &mut Child, deadline: Instant) -> Exit {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Exit::Exited(status.code()),
            Ok(None) => {}
            Err(error) => return Exit::Lost(error),
        }
        match deadline.checked_duration_since(Instant::now()) {
            Some(left) => thread::sleep(left.min(POLL)),
            None => {
                let _killed = child.kill();
                let _reaped = child.wait();
                return Exit::Stopped;
            }
        }
    }
}

/// Reads a pipe to its end on its own thread.
fn drain<R: Read + Send + 'static>(mut pipe: R) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    // A thread that cannot start drops the pipe and the sender, which reads as
    // no output: git then sees a closed pipe, not a full one.
    let _started = thread::Builder::new()
        .name("eph-git-output".to_string())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _read = pipe.read_to_end(&mut bytes);
            let _sent = sender.send(bytes);
        });
    receiver
}

/// A drained pipe's text, waited for until `until`, and whether it arrived.
fn collect(output: Option<&Receiver<Vec<u8>>>, until: Instant) -> (String, bool) {
    let bytes = output.and_then(|output| {
        output
            .recv_timeout(until.saturating_duration_since(Instant::now()))
            .ok()
    });
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
    fn a_deadline_beyond_what_an_instant_can_hold_is_clamped() {
        let dir = scratch("clamped");
        let result = git().with_timeout(Duration::MAX).run(&dir, &["--version"]);
        assert!(result.ok(), "{result:?}");
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

    fn write_atomic_in(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(path, text.as_bytes()).unwrap();
    }
}

//! POSIX process-group ownership for the child Ephesus launched (ADR-0042).
#![cfg(unix)]
#![expect(unsafe_code, reason = "ADR-0042: the native Unix lifecycle boundary")]

use std::{
    fs::File,
    io,
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::{CommandExt, ExitStatusExt},
    },
    process::{self, ChildStderr, ChildStdout, Command, Stdio},
    ptr::null_mut,
    sync::{Mutex, MutexGuard},
};

// openpty returns non-CLOEXEC descriptors. Serialize this short window with
// other launches in the process boundary so they cannot inherit those fds.
static SPAWN: Mutex<()> = Mutex::new(());

/// A child root and its dedicated process group. The root is owned
/// exclusively so a group id cannot be reused between observing it and
/// sending a signal.
#[must_use]
pub struct Child {
    root: process::Child,
    group: Option<libc::pid_t>,
    terminal: bool,
}

/// The root's exit status, retaining either its code or its signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    /// The program exited normally with this code.
    Code(i32),
    /// The kernel ended the program due to this signal.
    Signal(String),
}

/// A child started in its own group with null stdin and two output pipes.
pub struct Piped {
    /// Exclusively owned child and group.
    pub child: Child,
    /// Standard output, drained independently of stderr.
    pub stdout: ChildStdout,
    /// Standard error, drained independently of stdout.
    pub stderr: ChildStderr,
}

/// Whether the PTY master reported the kernel's end-of-stream error.
pub fn terminal_eof(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EIO)
}

/// Observe this fixture process's terminal geometry in cells.
#[cfg(any(test, feature = "test-support"))]
pub fn current_terminal_size() -> io::Result<(u16, u16)> {
    let mut size = dimensions(1, 1)?;
    // SAFETY: stdin is queried into an aligned, writable winsize. The kernel
    // checks that the descriptor belongs to a terminal.
    checked(unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &raw mut size) })?;
    Ok((size.ws_col, size.ws_row))
}

/// Start a caller-built command in a new process group with piped output.
pub fn spawn_piped(command: &mut Command) -> io::Result<Piped> {
    let _launch = launch_lock()?;
    let mut root = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let group = group_id(root.id())?;
    let pipes = root.stdout.take().zip(root.stderr.take());
    let mut child = Child {
        root,
        group: Some(group),
        terminal: false,
    };
    let Some((stdout, stderr)) = pipes else {
        child.terminate_group()?;
        return Err(io::Error::other(
            "process started without both output pipes",
        ));
    };
    Ok(Piped {
        child,
        stdout,
        stderr,
    })
}

/// The native PTY master retained for terminal geometry changes.
pub struct Console(File);

impl Console {
    /// Set terminal geometry in cells on this PTY.
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = dimensions(cols, rows)?;
        // SAFETY: the master is owned by self and the input describes a
        // complete winsize object valid for this call.
        checked(unsafe { libc::ioctl(self.0.as_raw_fd(), libc::TIOCSWINSZ, &raw const size) })
    }
}

/// The independent control, input and output handles of a POSIX terminal.
pub struct Terminal {
    /// Child and group ownership.
    pub child: Child,
    /// Bytes sent to the terminal's input line discipline.
    pub input: File,
    /// Output reader, including terminal control bytes.
    pub output: File,
    /// PTY master for resizing.
    pub console: Console,
}

/// Launch a caller-built command on an owned controlling terminal (ADR-0043).
/// The command's directory and environment are passed through as configured.
pub fn spawn_terminal(mut command: Command, cols: u16, rows: u16) -> io::Result<Terminal> {
    let mut size = dimensions(cols, rows)?;
    let _launch = launch_lock()?;
    let (mut master, mut slave) = (0, 0);
    // SAFETY: the return slots and initial size are valid. Passing null for
    // the optional name buffer and termios avoids buffer and ownership issues.
    checked(unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            null_mut(),
            null_mut(),
            &raw mut size,
        )
    })?;
    // SAFETY: success transfers ownership of both descriptors.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        // SAFETY: the descriptor is valid, and F_SETFD accepts integer flags.
        checked(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }
    let master = File::from(master);
    let input = master.try_clone()?;
    let output = master.try_clone()?;
    let slave = File::from(slave);
    command
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave);
    // SAFETY: std has installed the three standard handles when it invokes
    // pre_exec. The closure calls async-signal-safe functions with no
    // allocation or locks. setsid creates a fresh session/group, and stdin
    // is the open slave that becomes the controlling terminal.
    unsafe {
        command.pre_exec(|| {
            checked(libc::setsid())?;
            #[cfg(target_os = "macos")]
            let request = libc::c_ulong::from(libc::TIOCSCTTY);
            #[cfg(not(target_os = "macos"))]
            let request = libc::TIOCSCTTY;
            checked(libc::ioctl(libc::STDIN_FILENO, request, 0))
        });
    }
    let root = command.spawn()?;
    let group = group_id(root.id())?;
    Ok(Terminal {
        child: Child {
            root,
            group: Some(group),
            terminal: true,
        },
        input,
        output,
        console: Console(master),
    })
}

fn dimensions(cols: u16, rows: u16) -> io::Result<libc::winsize> {
    if cols == 0 || rows == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal dimensions must be positive",
        ));
    }
    Ok(libc::winsize {
        ws_col: cols,
        ws_row: rows,
        ws_xpixel: 0,
        ws_ypixel: 0,
    })
}

fn checked(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn launch_lock() -> io::Result<MutexGuard<'static, ()>> {
    SPAWN
        .lock()
        .map_err(|_| io::Error::other("native spawn gate is unavailable"))
}

impl Child {
    /// Check for root exit without collecting its status or releasing the PID
    /// that anchors the group. Use this while descendants can hold the pipes.
    pub fn exit_pending(&self) -> io::Result<bool> {
        match self.group {
            Some(pid) => exit_pending(pid),
            None => Ok(true),
        }
    }

    /// Wait for the root after a shutdown request, on a detached cleanup worker.
    /// This consumes the owner and can block until the system reports exit.
    pub fn reap(mut self) {
        let _reaped = self.root.wait();
    }

    /// Request forced shutdown of the owned group. Descendants that join
    /// another process group are outside the guarantee.
    ///
    /// Refuses once the child's status has been collected: in that state the
    /// numeric group id can be reused by the OS.
    pub fn terminate_group(&mut self) -> io::Result<()> {
        let group = self
            .group
            .ok_or_else(|| io::Error::other("the process-group owner has already been reaped"))?;
        // SAFETY: only this Child can reap the root. Its unreaped PID is the
        // id assigned to the dedicated group at creation, never zero or init.
        let result = unsafe { libc::kill(-group, libc::SIGKILL) };
        if result != 0 {
            return self.group_signal_failure(group, io::Error::last_os_error());
        }
        Ok(())
    }

    fn group_signal_failure(&self, _group: libc::pid_t, error: io::Error) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        if self.is_macos_zombie_group(_group, &error) {
            return Ok(());
        }
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn is_macos_zombie_group(&self, group: libc::pid_t, error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::EPERM)
            && self.exit_pending().unwrap_or(false)
            && group_contains_only_root(group)
    }

    /// Read the root's exit status when available. For a PTY, shut down any
    /// remaining group members before collecting the root. For a piped
    /// command the caller decides when to request group shutdown.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.terminal && self.group.is_some() {
            if !self.exit_pending()? {
                return Ok(None);
            }
            self.terminate_group()?;
        }
        let status = self.root.try_wait()?.map(|status| {
            if let Some(signal) = status.signal() {
                ExitStatus::Signal(signal.to_string())
            } else {
                ExitStatus::Code(status.code().unwrap_or_default())
            }
        });
        if status.is_some() {
            self.group = None;
        }
        Ok(status)
    }
}

#[cfg(target_os = "macos")]
fn group_contains_only_root(group: libc::pid_t) -> bool {
    // XNU's group signal skips zombies and can return EPERM when it finds
    // only the unreaped root. libproc lists live and zombie members. Two
    // slots distinguish a lone root from any other member or a failed query.
    let mut members = [0 as libc::pid_t; 2];
    // SAFETY: the aligned array is writable and its size is in bytes.
    let count = unsafe {
        libc::proc_listpgrppids(
            group,
            members.as_mut_ptr().cast(),
            size_of_val(&members) as libc::c_int,
        )
    };
    is_only_root(group, count, members)
}

#[cfg(any(target_os = "macos", test))]
fn is_only_root(group: libc::pid_t, count: libc::c_int, members: [libc::pid_t; 2]) -> bool {
    count == 1 && members[0] == group
}

fn exit_pending(pid: libc::pid_t) -> io::Result<bool> {
    // WNOHANG requires a zero si_pid on platforms that do not explicitly
    // write the returned structure when no child has exited.
    let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: WNOWAIT leaves the root unreaped and the output points to an
    // aligned siginfo_t slot. The positive pid belongs to this caller.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the slot was zeroed and waitid succeeded; this is the member
    // for the POSIX child-state notification.
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}

fn group_id(pid: u32) -> io::Result<libc::pid_t> {
    libc::pid_t::try_from(pid)
        .ok()
        .filter(|&pid| pid > 1)
        .ok_or_else(|| io::Error::other("invalid PID for a dedicated process group"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        io::{BufRead, BufReader, Write},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    const FIXTURE_DEPTH: &str = "EPH_GROUP_FIXTURE_DEPTH";
    const FIXTURE: [&str; 3] = ["--exact", "unix::tests::group_fixture", "--nocapture"];
    const SIZE_FIXTURE: &str = "EPH_POSIX_TEST_SIZE";
    const ESCAPE_GROUP: &str = "EPH_POSIX_TEST_JOIN_GROUP";

    #[test]
    fn native_error_and_pid_boundaries_preserve_failures() {
        assert!(terminal_eof(&io::Error::from_raw_os_error(libc::EIO)));
        assert!(!terminal_eof(&io::Error::from_raw_os_error(libc::EINTR)));
        assert!(!terminal_eof(&io::Error::other("unrelated read error")));
        checked(0).unwrap();
        checked(1).unwrap();
        assert!(checked(-1).is_err());
        assert_eq!(group_id(2).unwrap(), 2);
        assert!(group_id(0).is_err());
        assert!(group_id(1).is_err());
        assert!(group_id(u32::MAX).is_err());
        assert!(exit_pending(1).is_err(), "init is not a child of this test");
        assert!(dimensions(0, 1).is_err());
        assert!(dimensions(1, 0).is_err());
        let largest = dimensions(u16::MAX, u16::MAX).unwrap();
        assert_eq!((largest.ws_col, largest.ws_row), (u16::MAX, u16::MAX));
    }

    #[test]
    fn a_group_query_requires_exactly_one_member_with_the_root_identity() {
        assert!(is_only_root(37, 1, [37, 0]));
        for (count, members) in [(0, [37, 0]), (2, [37, 38]), (1, [38, 0]), (-1, [37, 0])] {
            assert!(!is_only_root(37, count, members));
        }
    }

    #[test]
    fn geometry_fixture() {
        if env::var_os(SIZE_FIXTURE).is_none() {
            return;
        }
        let mut line = String::new();
        io::stdin().read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "sample");
        let (cols, rows) = current_terminal_size().unwrap();
        println!("\nobserved-geometry={cols}x{rows}");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: launch a local fixture that reads geometry from its controlling terminal"
    )]
    fn posix_child_observes_the_requested_size_after_resize() {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", "unix::tests::geometry_fixture", "--nocapture"])
            .env_clear()
            .env(SIZE_FIXTURE, "1");
        if let Some(profile) = env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let Terminal {
            mut child,
            mut input,
            output,
            console,
        } = spawn_terminal(command, 80, 24).unwrap();
        let (sent, observed) = mpsc::channel();
        let drain = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) if line.starts_with("observed-geometry=") => {
                        sent.send(line).unwrap();
                        return;
                    }
                    Ok(_) => {}
                    Err(error) if terminal_eof(&error) => return,
                    Err(error) => panic!("fixture output failed: {error}"),
                }
            }
        });
        console.resize(97, 29).unwrap();
        input.write_all(b"sample\r").unwrap();
        assert_eq!(
            observed.recv_timeout(Duration::from_secs(3)).unwrap(),
            "observed-geometry=97x29"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(exit) = child.try_wait().unwrap() {
                assert_eq!(exit, ExitStatus::Code(0));
                break;
            }
            assert!(Instant::now() < deadline, "fixture did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        drain.join().unwrap();
        assert_eq!(child.try_wait().unwrap(), Some(ExitStatus::Code(0)));
        assert!(child.exit_pending().unwrap());
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: create live descendants inside the group being tested"
    )]
    fn group_fixture() {
        let Ok(depth) = env::var(FIXTURE_DEPTH) else {
            return;
        };
        // SAFETY: this runs in the disposable fixture process. SIG_IGN is a
        // POSIX-defined disposition that descendants inherit across exec.
        assert_ne!(
            unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) },
            libc::SIG_ERR
        );
        let depth: u8 = depth.parse().unwrap();
        if let Ok(group) = env::var(ESCAPE_GROUP) {
            let group: libc::pid_t = group.parse().unwrap();
            // SAFETY: this disposable child joins another test-owned group
            // in the same session. It never signals that group.
            checked(unsafe { libc::setpgid(0, group) }).unwrap();
        }
        let mut child = if depth < 2 {
            Some(
                Command::new(env::current_exe().unwrap())
                    .args(FIXTURE)
                    .env(FIXTURE_DEPTH, (depth + 1).to_string())
                    .spawn()
                    .unwrap(),
            )
        } else {
            None
        };
        println!("\neph group member {depth}");
        thread::sleep(Duration::from_secs(10));
        if let Some(child) = &mut child {
            assert!(child.wait().unwrap().success());
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: launch a three-generation group and prove its output pipe closes"
    )]
    fn forced_shutdown_stops_all_three_members_even_when_hangup_is_ignored() {
        let mut command = Command::new(env::current_exe().unwrap());
        command.args(FIXTURE).env(FIXTURE_DEPTH, "0");
        let Piped {
            mut child,
            stdout,
            stderr: _stderr,
        } = spawn_piped(&mut command).unwrap();
        let (ready, observations) = mpsc::channel();
        let drain = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if let Some(depth) = line.unwrap().strip_prefix("eph group member ") {
                    ready.send(depth.parse::<u8>().unwrap()).unwrap();
                }
            }
        });
        let mut seen = [false; 3];
        for _ in 0..seen.len() {
            seen[usize::from(observations.recv_timeout(Duration::from_secs(3)).unwrap())] = true;
        }
        assert_eq!(seen, [true; 3]);
        #[cfg(target_os = "macos")]
        assert!(!group_contains_only_root(child.group.unwrap()));
        assert_eq!(child.try_wait().unwrap(), None);
        assert!(!child.exit_pending().unwrap());
        child.terminate_group().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "the group root is still running");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status, ExitStatus::Signal(libc::SIGKILL.to_string()));
        assert!(child.exit_pending().unwrap());
        assert_eq!(
            observations.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Disconnected),
            "a surviving group member would still hold the pipe"
        );
        assert!(
            child.terminate_group().is_err(),
            "a reaped PID can no longer be signaled"
        );
        drain.join().unwrap();
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: prove that background disposal collects an actual child status"
    )]
    fn reaping_a_terminated_owned_child_leaves_no_waitable_root() {
        let mut command = Command::new(env::current_exe().unwrap());
        command.args(FIXTURE).env(FIXTURE_DEPTH, "2");
        let Piped {
            mut child,
            stdout,
            stderr,
        } = spawn_piped(&mut command).unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while !line.contains("eph group member 2") {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        }
        let pid = child.root.id() as libc::pid_t;
        assert!(!child.exit_pending().unwrap());
        child.terminate_group().unwrap();
        child.reap();
        assert!(
            exit_pending(pid).is_err(),
            "the root status was already collected"
        );
        drop(reader);
        drop(stderr);
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: prove ESRCH and the approved limit with a root that leaves its owned group"
    )]
    fn a_root_that_changes_groups_survives_shutdown_of_its_original_group() {
        let mut recipient_command = Command::new(env::current_exe().unwrap());
        recipient_command.args(FIXTURE).env(FIXTURE_DEPTH, "2");
        let Piped {
            child: mut recipient,
            stdout: recipient_stdout,
            stderr: _recipient_stderr,
        } = spawn_piped(&mut recipient_command).unwrap();
        let mut recipient_reader = BufReader::new(recipient_stdout);
        let mut ready = String::new();
        while !ready.contains("eph group member 2") {
            ready.clear();
            assert_ne!(recipient_reader.read_line(&mut ready).unwrap(), 0);
        }
        let recipient_group = recipient.group.unwrap();
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(FIXTURE)
            .env(FIXTURE_DEPTH, "2")
            .env(ESCAPE_GROUP, recipient_group.to_string());
        let Piped {
            mut child,
            stdout,
            stderr,
        } = spawn_piped(&mut command).unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while !line.contains("eph group member 2") {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        }
        let group = child.group.unwrap();
        assert!(!child.exit_pending().unwrap());
        // SAFETY: getpgid only observes the still-owned root's current group.
        assert_eq!(unsafe { libc::getpgid(group) }, recipient_group);
        child
            .group_signal_failure(group, io::Error::from_raw_os_error(libc::ESRCH))
            .unwrap();
        for code in [libc::EINVAL, libc::EPERM] {
            assert_eq!(
                child
                    .group_signal_failure(group, io::Error::from_raw_os_error(code))
                    .unwrap_err()
                    .raw_os_error(),
                Some(code)
            );
        }
        child.terminate_group().unwrap();
        assert!(
            !child.exit_pending().unwrap(),
            "the escaped root is outside the guarantee"
        );
        child.root.kill().unwrap();
        assert!(!child.root.wait().unwrap().success());
        recipient.terminate_group().unwrap();
        recipient.reap();
        drop(reader);
        drop(stderr);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: exercise Darwin's EPERM classification against live and zombie native groups"
    )]
    fn darwin_permission_exception_requires_an_exited_root_alone_and_eperm() {
        let mut command = Command::new(env::current_exe().unwrap());
        command.args(FIXTURE).env(FIXTURE_DEPTH, "2");
        let Piped {
            mut child,
            stdout,
            stderr,
        } = spawn_piped(&mut command).unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        while !line.contains("eph group member 2") {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
        }
        let group = child.group.unwrap();
        let eperm = io::Error::from_raw_os_error(libc::EPERM);
        assert!(group_contains_only_root(group));
        assert!(!child.is_macos_zombie_group(group, &eperm));
        child.root.kill().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !child.exit_pending().unwrap() {
            assert!(Instant::now() < deadline, "root did not become waitable");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(group_contains_only_root(group));
        assert!(child.is_macos_zombie_group(group, &eperm));
        assert!(!child.is_macos_zombie_group(group, &io::Error::from_raw_os_error(libc::EINVAL)));
        child.terminate_group().unwrap();
        child.reap();
        assert!(!group_contains_only_root(group));
        drop(reader);
        drop(stderr);
    }
}

//! Noninteractive engine probes with their own process lifetime (ADR-0042/0044).

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use eph_process::unix as native;
#[cfg(windows)]
use eph_process::windows as native;
use eph_pty::{
    ExitStatus,
    path::{find_executable, resolve_executable},
};
use thiserror::Error;

use crate::{Authentication, AuthenticationSpec, BinarySpec, CommandSpec};

const DEADLINE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(4);
const CLEANUP: Duration = Duration::from_millis(500);
const PIPE_LIMIT: u64 = 1024 * 1024;

/// A command completed, with its observed status and the answer from stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeCompletion<T> {
    /// The engine's process result, including a nonzero or signaled exit.
    pub status: ExitStatus,
    /// The value the adapter can establish from the output.
    pub answer: T,
}

/// The version and exact executable found by an acknowledged version probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExecutable {
    path: PathBuf,
    version: String,
}

impl VerifiedExecutable {
    /// The resolved path to use for authentication and the terminal.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The release reported by this executable.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// What version preflight can establish about a declared engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionCheck {
    /// Every PATH candidate was inspected and the executable was absent.
    Missing,
    /// The executable completed successfully with a recognized version.
    Ready(VerifiedExecutable),
    /// The executable ran but did not report a usable version.
    Unrecognized(ExitStatus),
}

/// A probe that did not return a complete result.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// PATH inspection could not reliably establish whether a candidate exists.
    #[error("could not inspect the engine probe's executable: {0}")]
    Inspect(#[source] io::Error),
    /// The command could not start in its specified environment.
    #[error("could not start the engine probe: {0}")]
    Start(#[source] io::Error),
    /// One stream failed or exceeded the retained output limit.
    #[error("could not capture engine probe {stream}: {source}")]
    Output {
        /// Which stream failed.
        stream: &'static str,
        /// The I/O or size failure.
        source: io::Error,
    },
    /// The OS could not establish a process result.
    #[error("could not observe the engine probe status: {0}")]
    Status(#[source] io::Error),
    /// The child or its output did not complete before the deadline.
    #[error("the engine probe exceeded its deadline")]
    Deadline,
    /// Shutdown could not be confirmed within the cleanup interval.
    #[error("{failure}; engine probe shutdown could not be confirmed: {source}")]
    Cleanup {
        /// The original probe failure.
        failure: Box<ProbeError>,
        /// Why shutdown could not be confirmed.
        source: io::Error,
    },
}

impl BinarySpec {
    /// Inspect PATH, probe the exact match, and distinguish absence from failure.
    pub fn check_version(
        self,
        cwd: &Path,
        environment: &[(OsString, OsString)],
    ) -> Result<VersionCheck, ProbeError> {
        let Some(path) = find_executable(OsStr::new(self.version.program), cwd, environment)
            .map_err(ProbeError::Inspect)?
        else {
            return Ok(VersionCheck::Missing);
        };
        let completed = command_at(&path, self.version, cwd, environment)?;
        if completed.status == ExitStatus::Code(0)
            && let Some(version) = self.parse_version(&completed.stdout)
        {
            return Ok(VersionCheck::Ready(VerifiedExecutable {
                path,
                version: version.to_owned(),
            }));
        }
        Ok(VersionCheck::Unrecognized(completed.status))
    }

    /// Run the recorded version command, using only the supplied environment.
    pub fn run_version(
        self,
        cwd: &Path,
        environment: &[(OsString, OsString)],
    ) -> Result<ProbeCompletion<Option<String>>, ProbeError> {
        let complete = command(self.version, cwd, environment)?;
        let answer = match complete.status {
            ExitStatus::Code(0) => self.parse_version(&complete.stdout).map(str::to_owned),
            _ => None,
        };
        Ok(ProbeCompletion {
            status: complete.status,
            answer,
        })
    }
}

impl AuthenticationSpec {
    /// Run the private auth probe with the exact executable verified earlier.
    pub fn run_at(
        self,
        executable: &VerifiedExecutable,
        cwd: &Path,
        environment: &[(OsString, OsString)],
    ) -> Result<ProbeCompletion<Authentication>, ProbeError> {
        let complete = command_at(executable.path(), self.probe, cwd, environment)?;
        Ok(ProbeCompletion {
            status: complete.status,
            answer: self.classify(&complete.stdout),
        })
    }

    /// Run the auth probe in the engine environment for this hire.
    pub fn run(
        self,
        cwd: &Path,
        environment: &[(OsString, OsString)],
    ) -> Result<ProbeCompletion<Authentication>, ProbeError> {
        let complete = command(self.probe, cwd, environment)?;
        let answer = self.classify(&complete.stdout);
        Ok(ProbeCompletion {
            status: complete.status,
            answer,
        })
    }
}

fn command(
    spec: CommandSpec,
    cwd: &Path,
    environment: &[(OsString, OsString)],
) -> Result<Completed, ProbeError> {
    let args: Vec<OsString> = spec.args.iter().map(OsString::from).collect();
    run(OsStr::new(spec.program), &args, cwd, environment, DEADLINE)
}

fn command_at(
    executable: &Path,
    spec: CommandSpec,
    cwd: &Path,
    environment: &[(OsString, OsString)],
) -> Result<Completed, ProbeError> {
    let cwd = std::path::absolute(cwd).map_err(ProbeError::Start)?;
    let args: Vec<OsString> = spec.args.iter().map(OsString::from).collect();
    run_resolved(executable, &args, &cwd, environment, DEADLINE)
}

struct Completed {
    status: ExitStatus,
    stdout: Vec<u8>,
}

fn run(
    program: &OsStr,
    args: &[OsString],
    cwd: &Path,
    environment: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<Completed, ProbeError> {
    let cwd = std::path::absolute(cwd).map_err(ProbeError::Start)?;
    let resolved = resolve_executable(program, &cwd, environment);
    if !resolved.is_absolute() {
        return Err(ProbeError::Start(io::Error::new(
            io::ErrorKind::NotFound,
            "program was not found on the probe plan's PATH",
        )));
    }
    run_resolved(&resolved, args, &cwd, environment, timeout)
}

fn run_resolved(
    executable: &Path,
    args: &[OsString],
    cwd: &Path,
    environment: &[(OsString, OsString)],
    timeout: Duration,
) -> Result<Completed, ProbeError> {
    let deadline = Instant::now() + timeout;
    let native::Piped {
        mut child,
        stdout,
        stderr,
    } = launch(executable, args, cwd, environment).map_err(ProbeError::Start)?;
    let finish = |child: &mut native::Child| {
        let stdout = drain(stdout).map_err(|source| ProbeError::Output {
            stream: "stdout",
            source,
        })?;
        let stderr = drain(stderr).map_err(|source| ProbeError::Output {
            stream: "stderr",
            source,
        })?;
        wait_complete(child, &stdout, &stderr, deadline)
    };
    match finish(&mut child) {
        Ok(complete) => Ok(complete),
        Err(failure) => {
            #[cfg(windows)]
            let terminate = |child: &mut native::Child| child.terminate();
            #[cfg(unix)]
            let terminate = native::Child::terminate_group;
            #[cfg(windows)]
            let job_empty =
                |child: &native::Child| child.active_processes().map(|count| count == 0);
            #[cfg(unix)]
            let job_empty = |_: &native::Child| Ok(true);

            let confirmed = stop(
                &mut child,
                terminate,
                job_empty,
                |child| Ok(status(child)?.is_some()),
                Instant::now,
                thread::sleep,
            );
            #[cfg(unix)]
            let confirmed = if confirmed.is_err() {
                thread::Builder::new()
                    .spawn(move || child.reap())
                    .and(confirmed)
            } else {
                confirmed
            };
            match confirmed {
                Ok(()) => Err(failure),
                Err(source) => Err(ProbeError::Cleanup {
                    failure: Box::new(failure),
                    source,
                }),
            }
        }
    }
}

#[cfg_attr(
    unix,
    expect(
        clippy::disallowed_methods,
        reason = "process: ADR-0044 allows engine probes through the owned native process boundary"
    )
)]
fn launch(
    executable: &Path,
    args: &[OsString],
    cwd: &Path,
    environment: &[(OsString, OsString)],
) -> io::Result<native::Piped> {
    #[cfg(windows)]
    return native::spawn_piped(native::Launch {
        executable,
        args,
        cwd,
        environment,
    });
    #[cfg(unix)]
    {
        let mut command = std::process::Command::new(executable);
        command.args(args).current_dir(cwd).env_clear();
        command.envs(environment.iter().map(|(key, value)| (key, value)));
        native::spawn_piped(&mut command)
    }
}

fn drain(pipe: impl Read + Send + 'static) -> io::Result<Receiver<io::Result<Vec<u8>>>> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new().spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe
            .take(PIPE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .and_then(|size| {
                if size as u64 > PIPE_LIMIT {
                    Err(io::Error::other("the probe stream exceeded 1 MiB"))
                } else {
                    Ok(bytes)
                }
            });
        let _delivered = sender.send(result);
    })?;
    Ok(receiver)
}

fn received(
    pipe: &Receiver<io::Result<Vec<u8>>>,
    stream: &'static str,
) -> Result<Option<Vec<u8>>, ProbeError> {
    let answer = match pipe.try_recv() {
        Ok(result) => result.map(Some),
        Err(TryRecvError::Empty) => return Ok(None),
        Err(TryRecvError::Disconnected) => Err(io::Error::other("output reader stopped")),
    };
    answer.map_err(|source| ProbeError::Output { stream, source })
}

fn wait_complete(
    child: &mut native::Child,
    stdout: &Receiver<io::Result<Vec<u8>>>,
    stderr: &Receiver<io::Result<Vec<u8>>>,
    deadline: Instant,
) -> Result<Completed, ProbeError> {
    let mut output = None;
    let mut stderr_done = false;
    #[cfg(windows)]
    let completed_status = status;
    #[cfg(unix)]
    let mut completed_status = {
        let mut shutdown_requested = false;
        move |child: &mut native::Child| {
            if !shutdown_requested {
                if !child.exit_pending()? {
                    return Ok(None);
                }
                // The unreaped root pins the process group ID until shutdown.
                child.terminate_group()?;
                shutdown_requested = true;
            }
            status(child)
        }
    };
    loop {
        if output.is_none() {
            output = received(stdout, "stdout")?;
        }
        if !stderr_done {
            stderr_done = received(stderr, "stderr")?.is_some();
        }
        if stderr_done
            && output.is_some()
            && let Some(status) = completed_status(child).map_err(ProbeError::Status)?
            && let Some(stdout) = output.take()
        {
            return Ok(Completed { status, stdout });
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ProbeError::Deadline);
        }
        thread::sleep(remaining.min(POLL));
    }
}

fn status(child: &mut native::Child) -> io::Result<Option<ExitStatus>> {
    #[cfg(windows)]
    return Ok(child.try_wait()?.map(ExitStatus::Code));
    #[cfg(unix)]
    {
        if !child.exit_pending()? {
            return Ok(None);
        }
        Ok(child.try_wait()?.map(|status| match status {
            native::ExitStatus::Code(code) => ExitStatus::Code(code as u32),
            native::ExitStatus::Signal(signal) => ExitStatus::Signal(signal),
        }))
    }
}

fn stop<T>(
    child: &mut T,
    terminate: impl FnOnce(&mut T) -> io::Result<()>,
    mut job_empty: impl FnMut(&T) -> io::Result<bool>,
    mut root_exited: impl FnMut(&mut T) -> io::Result<bool>,
    mut now: impl FnMut() -> Instant,
    mut pause: impl FnMut(Duration),
) -> io::Result<()> {
    terminate(child)?;
    let until = now() + CLEANUP;
    loop {
        if job_empty(child)? && root_exited(child)? {
            return Ok(());
        }
        let left = until.saturating_duration_since(now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "exit was not observed",
            ));
        }
        pause(left.min(POLL));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::{env, spec::CLAUDE, test_support::script};
    use eph_fs::atomic::write_atomic;

    const FIXTURE: &str = "EPH_ENGINE_PROBE_FIXTURE";
    const READY: &str = "EPH_ENGINE_PROBE_READY";

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: the probe fixtures start owned descendants to observe cleanup"
    )]
    fn child_fixture() {
        match std::env::var(FIXTURE).as_deref() {
            Ok("report") => {
                println!("fixture stdout");
                eprintln!("fixture stderr");
                std::process::exit(17);
            }
            Ok("hold") => {
                let marker = std::env::var_os(READY).unwrap();
                write_atomic(Path::new(&marker), b"descendant got to sleep").unwrap();
                thread::sleep(Duration::from_secs(3));
            }
            Ok("descendant") => {
                let _child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "probe::tests::child_fixture", "--nocapture"])
                    .env(FIXTURE, "hold")
                    .spawn()
                    .unwrap();
                std::process::exit(0);
            }
            #[cfg(unix)]
            Ok("descendant-without-pipes") => {
                let _child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "probe::tests::child_fixture", "--nocapture"])
                    .env(FIXTURE, "liveness-peer")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap();
                let ready = std::env::var_os(READY).unwrap();
                let until = Instant::now() + Duration::from_secs(3);
                while !Path::new(&ready).is_file() {
                    assert!(Instant::now() < until, "descendant did not connect");
                    thread::sleep(POLL);
                }
                std::process::exit(17);
            }
            #[cfg(unix)]
            Ok("liveness-peer") => {
                use std::os::unix::net::UnixStream;

                let ready = std::path::PathBuf::from(std::env::var_os(READY).unwrap());
                let mut connection = UnixStream::connect(ready.with_extension("sock")).unwrap();
                write_atomic(&ready, b"descendant connected").unwrap();
                let _closed = connection.read(&mut [0]);
            }
            _ => {}
        }
    }

    fn fixture(mode: &str, dir: &Path, timeout: Duration) -> Result<Completed, ProbeError> {
        let exe = std::env::current_exe().unwrap();
        let mut env = env::base_environment(&std::env::vars_os().collect::<Vec<_>>());
        env.extend([
            (FIXTURE.into(), mode.into()),
            (READY.into(), dir.join("ready").into_os_string()),
        ]);
        run(
            exe.as_os_str(),
            &[
                "--exact".into(),
                "probe::tests::child_fixture".into(),
                "--nocapture".into(),
            ],
            dir,
            &env,
            timeout,
        )
    }

    #[test]
    fn a_completed_nonzero_command_keeps_its_output() {
        let root = tempfile::tempdir().unwrap();
        let completed = fixture("report", root.path(), Duration::from_secs(5)).unwrap();
        assert_eq!(completed.status, ExitStatus::Code(17));
        assert!(
            std::str::from_utf8(&completed.stdout)
                .unwrap()
                .contains("fixture stdout")
        );
    }

    #[test]
    fn a_descendant_holding_the_pipes_keeps_the_probe_incomplete() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            fixture("descendant", root.path(), Duration::from_secs(1)),
            Err(ProbeError::Deadline)
        ));
        assert_eq!(
            std::fs::read(root.path().join("ready")).unwrap(),
            b"descendant got to sleep"
        );
    }

    #[test]
    fn forced_shutdown_waits_for_the_job_and_root() {
        let start = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let mut requested = false;
        let mut job = [false, true, true].into_iter();
        let mut root = [false, true].into_iter();
        stop(
            &mut requested,
            |requested| {
                *requested = true;
                Ok(())
            },
            |requested| {
                assert!(*requested);
                Ok(job.next().unwrap())
            },
            |_| Ok(root.next().unwrap()),
            || start + elapsed.get(),
            |delay| elapsed.set(elapsed.get() + delay),
        )
        .unwrap();
        assert_eq!(elapsed.get(), Duration::from_millis(8));
        assert!(job.next().is_none());
        assert!(root.next().is_none());
    }

    #[test]
    fn forced_shutdown_reports_the_cleanup_deadline() {
        let start = Instant::now();
        let mut elapsed = [0, 499, 500].map(Duration::from_millis).into_iter();
        let mut pauses = Vec::new();
        let error = stop(
            &mut (),
            |_| Ok(()),
            |_| Ok(true),
            |_| Ok(false),
            || start + elapsed.next().unwrap(),
            |delay| pauses.push(delay),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(pauses, [Duration::from_millis(1)]);
        assert!(elapsed.next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_completed_probe_stops_its_descendant_after_both_pipes_close() {
        use std::{net::Shutdown, os::unix::net::UnixListener};

        let root = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(root.path().join("ready.sock")).unwrap();
        let completed = fixture(
            "descendant-without-pipes",
            root.path(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(completed.status, ExitStatus::Code(17));
        assert_eq!(
            std::fs::read(root.path().join("ready")).unwrap(),
            b"descendant connected"
        );
        let (mut descendant, _) = listener.accept().unwrap();
        let cleanup = descendant.try_clone().unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            let _sent = send.send(descendant.read(&mut [0]));
        });
        let read = receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_else(|error| {
                let _shutdown = cleanup.shutdown(Shutdown::Both);
                panic!("descendant socket did not report EOF: {error}");
            });
        reader.join().unwrap();
        assert_eq!(read.unwrap(), 0);
    }

    #[test]
    fn the_public_probes_replay_captured_bytes_through_real_processes() {
        let root = tempfile::tempdir().unwrap();
        script(
            root.path(),
            "eph-probe-replay",
            "@echo off\r\ntype \"%EPH_PROBE_OUTPUT%\"\r\nexit /b %EPH_PROBE_EXIT%\r\n",
            "#!/bin/sh\n/bin/cat \"$EPH_PROBE_OUTPUT\"\nexit \"$EPH_PROBE_EXIT\"\n",
        );
        let mut environment = env::base_environment(&std::env::vars_os().collect::<Vec<_>>());
        let bytes = root.path().join("fixture.stdout");
        env::put(&mut environment, "PATH", root.path());
        env::put(&mut environment, "EPH_PROBE_OUTPUT", bytes.as_path());
        let mut version = CLAUDE;
        version.version.program = "eph-probe-replay";
        write_atomic(
            &bytes,
            include_bytes!("../tests/fixtures/engine-output/claude/version.txt"),
        )
        .unwrap();
        env::put(&mut environment, "EPH_PROBE_EXIT", "0");
        assert_eq!(
            version.run_version(root.path(), &environment).unwrap(),
            ProbeCompletion {
                status: ExitStatus::Code(0),
                answer: Some("2.1.295".to_owned())
            }
        );
        let VersionCheck::Ready(executable) =
            version.check_version(root.path(), &environment).unwrap()
        else {
            panic!("expected the recorded version to identify the executable");
        };
        assert!(executable.path().is_absolute());
        assert_eq!(executable.version(), "2.1.295");
        env::put(&mut environment, "EPH_PROBE_EXIT", "17");
        assert_eq!(
            version.run_version(root.path(), &environment).unwrap(),
            ProbeCompletion {
                status: ExitStatus::Code(17),
                answer: None
            }
        );
        assert_eq!(
            version.check_version(root.path(), &environment).unwrap(),
            VersionCheck::Unrecognized(ExitStatus::Code(17)),
        );
        let mut auth = version.authentication.unwrap();
        auth.probe.program = "eph-probe-replay";
        write_atomic(
            &bytes,
            include_bytes!("../tests/fixtures/engine-output/claude/auth-status-logged-out.json"),
        )
        .unwrap();
        env::put(&mut environment, "EPH_PROBE_EXIT", "1");
        assert_eq!(
            auth.run(root.path(), &environment).unwrap(),
            ProbeCompletion {
                status: ExitStatus::Code(1),
                answer: Authentication::NeedsLogin
            }
        );
        env::put(&mut environment, "PATH", "");
        assert_eq!(
            auth.run_at(&executable, root.path(), &environment)
                .unwrap()
                .answer,
            Authentication::NeedsLogin,
        );
        assert_eq!(
            version.check_version(root.path(), &environment).unwrap(),
            VersionCheck::Missing
        );
        env::put(&mut environment, "PATH", root.path());
        std::fs::create_dir(root.path().join("not-an-executable.exe")).unwrap();
        version.version.program = "not-an-executable.exe";
        assert!(matches!(
            version.check_version(root.path(), &environment),
            Err(ProbeError::Inspect(_))
        ));
    }

    #[test]
    fn an_absent_program_never_falls_back_to_the_parent_path() {
        assert!(matches!(
            run(
                OsStr::new("ephesus-probe-fixture-not-installed"),
                &[],
                Path::new("."),
                &[],
                DEADLINE,
            ),
            Err(ProbeError::Start(error)) if error.kind() == io::ErrorKind::NotFound
        ));
    }

    #[test]
    fn the_probe_distinguishes_pipe_failure_from_an_empty_capture() {
        let (write, read) = mpsc::channel();
        assert!(received(&read, "stdout").unwrap().is_none());
        write.send(Ok(Vec::new())).unwrap();
        assert_eq!(received(&read, "stdout").unwrap(), Some(Vec::new()));
        drop(write);
        assert!(matches!(
            received(&read, "stderr"),
            Err(ProbeError::Output {
                stream: "stderr",
                ..
            })
        ));
    }

    #[test]
    fn the_probe_stream_accepts_one_mib_and_refuses_the_next_byte() {
        let maximum = 1_048_576;
        let at_limit = drain(io::repeat(b'a').take(maximum)).unwrap();
        let over_limit = drain(io::repeat(b'a').take(maximum + 1)).unwrap();
        assert_eq!(
            at_limit
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap()
                .len(),
            maximum as usize
        );
        assert!(
            over_limit
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .is_err()
        );
    }
}

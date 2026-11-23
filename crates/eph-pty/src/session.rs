//! Spawn, control and drain an owned terminal (FR-1.1, ADR-0014, ADR-0042).

use crate::{path, redaction::Redactor};
#[cfg(unix)]
use eph_process::unix as native;
#[cfg(windows)]
use eph_process::windows as native;
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc, RwLock,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender},
    },
    thread,
    time::Duration,
};

const OUTPUT_CHUNKS: usize = 16;
const READ_BYTES: usize = 4096;

/// The exact execution plan supplied by the caller. Its environment is never
/// extended from the harness. The argument list follows the executable name.
pub struct SpawnPlan<'a> {
    /// A path or a program resolved using the plan's PATH.
    pub executable: &'a OsStr,
    /// Program arguments as individual values.
    pub args: &'a [OsString],
    /// Working directory for the child.
    pub cwd: &'a Path,
    /// Full child environment, including any authorized secret grants.
    pub environment: &'a [(OsString, OsString)],
    /// Terminal width in cells.
    pub cols: u16,
    /// Terminal height in cells.
    pub rows: u16,
}

/// Root exit status, with normal codes and POSIX signals kept distinct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    /// The program returned an exit code.
    Code(u32),
    /// A POSIX signal ended the program.
    Signal(String),
}

/// The operating-system boundary to which forced shutdown was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownScope {
    /// All members of the Windows Job Object.
    WindowsJob,
    /// Members of the launched POSIX process group. Descendants in other
    /// groups are outside this guarantee.
    PosixProcessGroup,
}

/// An observation of the redacted terminal stream.
#[derive(Debug, PartialEq, Eq)]
pub enum Output {
    /// The next chunk of safe terminal bytes. Terminal control bytes are preserved.
    Data(Vec<u8>),
    /// No data was available before the requested timeout.
    Pending,
    /// The stream ended. All redactor carry has been delivered.
    End,
}

/// One owned terminal. Drain its bounded output queue while the process runs.
///
/// Poll root exit separately from output, and continue draining to
/// [`Output::End`] for the final bytes. Dropping the session requests cleanup
/// in the background and discards any output the caller has left unread.
#[must_use]
pub struct Session {
    backend: Option<Backend>,
    cleanup: Sender<(Backend, bool)>,
    output: Option<Receiver<io::Result<Vec<u8>>>>,
    values: Arc<RwLock<Values>>,
    exit: Option<ExitStatus>,
}

struct Values {
    launch: Vec<String>,
    current: Vec<String>,
}

impl Session {
    /// Start the terminal and its redaction worker. Only supplied broker
    /// values are considered secrets. Workers are allocated before the child
    /// so thread startup failures cannot abandon a live process.
    pub fn spawn(plan: SpawnPlan<'_>, values: Vec<String>) -> io::Result<Self> {
        validate_size(plan.cols, plan.rows)?;
        let (cleanup, finish) = mpsc::channel::<(Backend, bool)>();
        let _cleanup_worker = thread::Builder::new()
            .name("eph-pty-cleanup".to_owned())
            .spawn(move || {
                if let Ok((backend, exited)) = finish.recv() {
                    backend.dispose(exited);
                }
            })?;

        let (attach, reader) = mpsc::channel::<Box<dyn Read + Send>>();
        let (sender, output) = mpsc::sync_channel(OUTPUT_CHUNKS);
        let values = Arc::new(RwLock::new(Values {
            launch: values,
            current: Vec::new(),
        }));
        let worker_values = Arc::clone(&values);
        let _output_worker = thread::Builder::new()
            .name("eph-pty-output".to_owned())
            .spawn(move || {
                if let Ok(reader) = reader.recv()
                    && let Err(error) = drain(reader, &worker_values, &sender)
                {
                    let _reported = sender.send(Err(error));
                }
                forget_values(&worker_values);
            })?;

        let (backend, reader) = Backend::spawn(plan)?;
        if attach.send(reader).is_err() {
            let _queued = cleanup.send((backend, false));
            return Err(io::Error::other("PTY output worker ended at startup"));
        }
        Ok(Self {
            backend: Some(backend),
            cleanup,
            output: Some(output),
            values,
            exit: None,
        })
    }

    /// Replace current broker values for the next chunk. Initial values
    /// remain masked through session exit (ADR-0043).
    pub fn set_redaction_values(&self, values: Vec<String>) -> io::Result<()> {
        let mut current = self.values.write().map_err(|_| redactor_unavailable())?;
        current.current = values;
        Ok(())
    }

    /// Write bytes to the child's terminal input, with its normal line discipline.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.running()?.input.write_all(bytes)
    }

    /// Send the engine adapter's interrupt key sequence (ADR-0009).
    pub fn interrupt(&mut self, sequence: &[u8]) -> io::Result<()> {
        self.write(sequence)
    }

    /// Change the terminal geometry in cells.
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        validate_size(cols, rows)?;
        self.running()?.resize(cols, rows)
    }

    /// Request termination of the owned OS boundary. An already observed
    /// exit returns the same scope without sending a new signal. Poll
    /// [`Self::try_wait`] for the observed root status.
    pub fn kill(&mut self) -> io::Result<ShutdownScope> {
        if let Some(backend) = &mut self.backend {
            backend.terminate()?;
        }
        #[cfg(windows)]
        let scope = ShutdownScope::WindowsJob;
        #[cfg(unix)]
        let scope = ShutdownScope::PosixProcessGroup;
        Ok(scope)
    }

    /// Check for root exit without blocking on terminal output. After observing
    /// exit, schedule close while the output worker can still drain the terminal.
    /// On macOS, an unconfirmed shutdown can return a timeout error while the
    /// session still owns the process group. Later polls can finish cleanup.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(backend) = &mut self.backend
            && let Some(exit) = backend.poll_exit()?
        {
            self.exit = Some(exit);
            if let Some(backend) = self.backend.take() {
                self.cleanup
                    .send((backend, true))
                    .map_err(|_| io::Error::other("PTY cleanup worker ended unexpectedly"))?;
            }
        }
        Ok(self.exit.clone())
    }

    /// Receive the next redacted chunk, terminal read error, timeout or EOF.
    pub fn output(&self, timeout: Duration) -> io::Result<Output> {
        let output = self.output.as_ref().ok_or_else(terminal_closed)?;
        match output.recv_timeout(timeout) {
            Ok(chunk) => chunk.map(Output::Data),
            Err(RecvTimeoutError::Timeout) => Ok(Output::Pending),
            Err(RecvTimeoutError::Disconnected) => Ok(Output::End),
        }
    }

    fn running(&mut self) -> io::Result<&mut Backend> {
        self.backend.as_mut().ok_or_else(terminal_closed)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Unblock a worker waiting on a full output queue before ConPTY closes.
        drop(self.output.take());
        forget_values(&self.values);
        if let Some(backend) = self.backend.take() {
            let _queued = self.cleanup.send((backend, false));
        }
    }
}

struct Backend {
    child: native::Child,
    input: Box<dyn Write + Send>,
    console: native::Console,
}

impl Backend {
    #[cfg_attr(
        unix,
        expect(
            clippy::disallowed_methods,
            reason = "process: construct the agent PTY spawn plan, then delegate OS creation to ADR-0043's boundary"
        )
    )]
    fn spawn(plan: SpawnPlan<'_>) -> io::Result<(Self, Box<dyn Read + Send>)> {
        let cwd = std::path::absolute(plan.cwd)?;
        if !cwd.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "PTY working directory is not a directory",
            ));
        }
        let executable = path::for_spawn(plan.executable, &cwd, plan.environment)?;
        #[cfg(windows)]
        let terminal = native::spawn_terminal(
            native::Launch {
                executable: &executable,
                args: plan.args,
                cwd: &cwd,
                environment: plan.environment,
            },
            plan.cols,
            plan.rows,
        )?;
        #[cfg(unix)]
        let terminal = {
            let mut command = std::process::Command::new(executable);
            command.args(plan.args).current_dir(cwd).env_clear();
            command.envs(plan.environment.iter().map(|(key, value)| (key, value)));
            native::spawn_terminal(command, plan.cols, plan.rows)?
        };
        Ok((
            Self {
                child: terminal.child,
                input: Box::new(terminal.input),
                console: terminal.console,
            },
            Box::new(terminal.output),
        ))
    }

    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.console.resize(cols, rows)
    }

    fn terminate(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        return self.child.terminate();
        #[cfg(unix)]
        self.child.terminate_group()
    }

    fn poll_exit(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(windows)]
        {
            let status = self.child.try_wait()?;
            if status.is_some() {
                self.child.terminate()?;
            }
            Ok(status.map(ExitStatus::Code))
        }
        #[cfg(unix)]
        Ok(self.child.try_wait()?.map(|status| match status {
            native::ExitStatus::Code(code) => ExitStatus::Code(code as u32),
            native::ExitStatus::Signal(signal) => ExitStatus::Signal(signal),
        }))
    }

    fn dispose(mut self, exited: bool) {
        if !exited {
            let _requested = self.terminate();
        }
        #[cfg(windows)]
        {
            // The job is closed before the console. The reader is independent.
            drop(self.child);
        }
        #[cfg(unix)]
        {
            drop(self.input);
            drop(self.console);
            // An observed exit is already cached by try_wait. Otherwise this
            // background worker collects the root after the shutdown request.
            self.child.reap();
        }
    }
}

fn drain(
    mut reader: Box<dyn Read + Send>,
    values: &RwLock<Values>,
    sender: &SyncSender<io::Result<Vec<u8>>>,
) -> io::Result<()> {
    let mut buf = [0; READ_BYTES];
    let mut filter = Redactor::default();
    loop {
        let count = match reader.read(&mut buf) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            #[cfg(unix)]
            Err(error) if native::terminal_eof(&error) => 0,
            Err(error) => return Err(error),
        };
        let redacted = {
            let current = values.read().map_err(|_| redactor_unavailable())?;
            let borrowed: Vec<_> = current
                .launch
                .iter()
                .chain(&current.current)
                .map(String::as_str)
                .collect();
            if count == 0 {
                filter.flush(&borrowed)
            } else {
                filter.push(&buf[..count], &borrowed)
            }
        };
        if !redacted.is_empty() && sender.send(Ok(redacted)).is_err() {
            return Ok(());
        }
        if count == 0 {
            return Ok(());
        }
    }
}

fn validate_size(cols: u16, rows: u16) -> io::Result<()> {
    if cols == 0 || rows == 0 || cols > i16::MAX as u16 || rows > i16::MAX as u16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal size must be 1–32767 cells on each axis",
        ));
    }
    Ok(())
}

fn redactor_unavailable() -> io::Error {
    io::Error::other("PTY redaction values are unavailable")
}

fn forget_values(values: &RwLock<Values>) {
    if let Ok(mut values) = values.write() {
        values.launch.clear();
        values.current.clear();
    }
}

fn terminal_closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "PTY session has ended")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, io::BufRead as _, time::Instant};

    const FIXTURE: &str = "EPH_PTY_TEST_VALUE";
    const VALUE: &str = "demo-tail";
    const PROFILE: &str = "LLVM_PROFILE_FILE";
    const PARITY_TIMEOUT: Duration = Duration::from_secs(5);

    // Invoked as the actual child in its PTY. The harness neither loads an
    // engine nor assumes anything about the operator's current shell.
    #[test]
    fn terminal_fixture() {
        let Ok(value) = env::var(FIXTURE) else {
            return;
        };
        let keys: Vec<_> = env::vars_os()
            .map(|(name, _)| name)
            .filter(|name| {
                // The coverage runner's path is explicitly included in this
                // test plan. The LLVM runtime adds its own init marker
                // inside instrumented children before this fixture starts.
                name != PROFILE && name != "__LLVM_PROFILE_RT_INIT_ONCE"
            })
            .collect();
        assert_eq!(keys, [FIXTURE], "the fixture's exact environment");
        let mut stdin = io::stdin().lock();
        let mut first = String::new();
        let mut second = String::new();
        println!("fixture-pid={}", std::process::id());
        println!("fixture-ready");
        stdin.read_line(&mut first).unwrap();
        let (cols, rows) = native::current_terminal_size().unwrap();
        println!("fixture-geometry={cols}x{rows}");
        println!("fixture-input={}", first.trim_end());
        stdin.read_line(&mut second).unwrap();
        println!("fixture-interrupt={}", second.trim_end());
        print!("fixture-value={value}; fresh-broker-value");
        io::stdout().flush().unwrap();
    }

    fn fixture() -> Session {
        let executable = env::current_exe().unwrap();
        let cwd = env::current_dir().unwrap();
        let args =
            ["--exact", "session::tests::terminal_fixture", "--nocapture"].map(OsString::from);
        let mut environment = vec![(FIXTURE.into(), VALUE.into())];
        if let Some(profile) = env::var_os(PROFILE) {
            environment.push((PROFILE.into(), profile));
        }
        Session::spawn(
            SpawnPlan {
                executable: executable.as_os_str(),
                args: &args,
                cwd: &cwd,
                environment: &environment,
                cols: 80,
                rows: 24,
            },
            vec!["demo".to_owned(), VALUE.to_owned()],
        )
        .unwrap()
    }

    fn until_ready(session: &mut Session) -> Vec<u8> {
        let timeout = Instant::now() + PARITY_TIMEOUT;
        let mut seen = Vec::new();
        while !String::from_utf8_lossy(&seen).contains("fixture-ready") {
            assert!(
                Instant::now() < timeout,
                "fixture failed to start: {:?}",
                String::from_utf8_lossy(&seen)
            );
            let _exit = session.try_wait().unwrap();
            match session.output(Duration::from_millis(20)).unwrap() {
                Output::Data(chunk) => seen.extend(chunk),
                Output::Pending => {}
                Output::End => panic!(
                    "term ended before fixture readiness: {:?}",
                    String::from_utf8_lossy(&seen)
                ),
            }
        }
        seen
    }

    fn finish(session: &mut Session, mut seen: Vec<u8>) -> (ExitStatus, String) {
        let deadline = Instant::now() + PARITY_TIMEOUT;
        let mut ended = false;
        loop {
            assert!(Instant::now() < deadline, "fixture never closed: {seen:?}");
            let exit = session.try_wait().unwrap();
            if ended {
                if let Some(exit) = exit {
                    return (exit, String::from_utf8_lossy(&seen).into_owned());
                }
                thread::sleep(Duration::from_millis(2));
            } else {
                match session.output(Duration::from_millis(10)).unwrap() {
                    Output::Data(chunk) => seen.extend(chunk),
                    Output::End => ended = true,
                    Output::Pending => {}
                }
            }
        }
    }

    #[test]
    fn real_terminal_delivers_input_resizes_and_redacts_through_the_public_session() {
        let mut session = fixture();
        let seen = until_ready(&mut session);
        session
            .set_redaction_values(vec!["fresh-broker-value".to_owned()])
            .unwrap();
        session.resize(101, 27).unwrap();
        assert!(session.resize(0, 27).is_err());
        session.write(b"typed\r").unwrap();
        session.interrupt(b"cancel\r").unwrap();
        let (exit, output) = finish(&mut session, seen);
        assert_eq!(exit, ExitStatus::Code(0));
        assert!(output.contains("fixture-input=typed"), "{output:?}");
        assert!(output.contains("fixture-geometry=101x27"), "{output:?}");
        assert!(output.contains("fixture-interrupt=cancel"), "{output:?}");
        assert!(
            output.contains("fixture-value=•••eph-masked•••"),
            "{output:?}"
        );
        assert!(!output.contains(VALUE));
        assert!(
            output.contains("•••eph-masked•••; •••eph-masked•••"),
            "{output:?}"
        );
        assert!(session.write(b"after exit").is_err());
        let retained = session.values.read().unwrap();
        assert!(retained.launch.is_empty() && retained.current.is_empty());
    }

    #[test]
    fn terminal_dimensions_accept_both_boundaries_and_refuse_each_invalid_axis() {
        for (cols, rows) in [(1, 1), (32767, 32767), (1, 32767), (32767, 1)] {
            validate_size(cols, rows).unwrap();
        }
        for (cols, rows) in [
            (0, 1),
            (1, 0),
            (32768, 1),
            (1, 32768),
            (u16::MAX, 1),
            (1, u16::MAX),
        ] {
            assert_eq!(
                validate_size(cols, rows).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn dropping_a_live_session_closes_its_owned_output_and_forgets_credentials() {
        let mut session = fixture();
        let ready = until_ready(&mut session);
        #[cfg(target_os = "linux")]
        let root = {
            let ready = String::from_utf8_lossy(&ready);
            let pid: u32 = ready
                .split("fixture-pid=")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let root = std::path::PathBuf::from(format!("/proc/{pid}"));
            assert!(root.metadata().unwrap().is_dir());
            root
        };
        #[cfg(not(target_os = "linux"))]
        drop(ready);
        let output = session.output.take().unwrap();
        let values = Arc::clone(&session.values);
        drop(session);
        let deadline = Instant::now() + PARITY_TIMEOUT;
        loop {
            assert!(
                Instant::now() < deadline,
                "dropped terminal still held its output"
            );
            match output.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) => {
                    chunk.unwrap();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        #[cfg(target_os = "linux")]
        loop {
            match root.metadata() {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => panic!("could not inspect fixture root: {error}"),
                Ok(_) => {
                    assert!(Instant::now() < deadline, "dropped root was not reaped");
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
        let retained = values.read().unwrap();
        assert!(retained.launch.is_empty() && retained.current.is_empty());
    }

    #[test]
    fn the_output_worker_retries_interruptions_and_reports_other_read_errors() {
        struct Chunks(std::collections::VecDeque<io::Result<Vec<u8>>>);

        impl Read for Chunks {
            fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
                let bytes = self.0.pop_front().unwrap_or_else(|| Ok(Vec::new()))?;
                target[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            }
        }

        let values = RwLock::new(Values {
            launch: Vec::new(),
            current: Vec::new(),
        });
        let (sent, received) = mpsc::sync_channel(8);
        let chunks = [
            Err(io::ErrorKind::Interrupted.into()),
            Ok(b"observed output".to_vec()),
            Err(io::Error::other("broken reader")),
        ];
        let error = drain(Box::new(Chunks(chunks.into())), &values, &sent).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(received.try_recv().unwrap().unwrap(), b"observed output");

        #[cfg(unix)]
        {
            let chunks = [Err(io::Error::from_raw_os_error(5))];
            drain(Box::new(Chunks(chunks.into())), &values, &sent).unwrap();
        }
    }
    #[test]
    fn kill_returns_the_os_scope_and_the_public_session_reports_exit_and_eof() {
        let mut session = fixture();
        let seen = until_ready(&mut session);
        let scope = session.kill().unwrap();
        #[cfg(windows)]
        assert_eq!(scope, ShutdownScope::WindowsJob);
        #[cfg(unix)]
        assert_eq!(scope, ShutdownScope::PosixProcessGroup);
        let (exit, _) = finish(&mut session, seen);
        assert_ne!(exit, ExitStatus::Code(0));
        assert_eq!(session.try_wait().unwrap(), Some(exit));
        assert_eq!(session.kill().unwrap(), scope);
    }

    #[test]
    fn a_missing_working_directory_refuses_an_absolute_executable() {
        let directory = tempfile::tempdir().unwrap();
        let absent = directory.path().join("missing");
        let executable = env::current_exe().unwrap();
        let result = Session::spawn(
            SpawnPlan {
                executable: executable.as_os_str(),
                args: &[],
                cwd: &absent,
                environment: &[],
                cols: 80,
                rows: 24,
            },
            Vec::new(),
        );
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::NotFound);
    }

    #[cfg(windows)]
    #[test]
    fn an_unknown_batch_shim_runs_in_conpty_and_receives_a_checked_argument() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("batch fixture.cmd");
        eph_fs::atomic::write_atomic(
            &script,
            b"@echo off\r\necho batch-ran:[%~1]\r\nexit /b 0\r\n",
        )
        .unwrap();
        let path = [(
            OsString::from("PATH"),
            directory.path().as_os_str().to_owned(),
        )];
        let mut session = Session::spawn(
            SpawnPlan {
                executable: OsStr::new("batch fixture.cmd"),
                args: &["value with spaces".into()],
                cwd: directory.path(),
                environment: &path,
                cols: 80,
                rows: 24,
            },
            Vec::new(),
        )
        .unwrap();
        let (status, output) = finish(&mut session, Vec::new());
        assert_eq!(status, ExitStatus::Code(0));
        assert!(
            output.contains("batch-ran:[value with spaces]"),
            "{output:?}"
        );

        let refused = Session::spawn(
            SpawnPlan {
                executable: script.as_os_str(),
                args: &["literal percent % mark".into()],
                cwd: directory.path(),
                environment: &[],
                cols: 80,
                rows: 24,
            },
            Vec::new(),
        );
        let error = refused
            .err()
            .expect("batch expansion characters are refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("underlying executable"));
    }
}

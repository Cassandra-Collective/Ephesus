//! R2.1 probe for the behavior of portable-pty's child kill on a real process tree.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const ROLES: [&str; 3] = ["root", "child", "grandchild"];
const INTERVAL: Duration = Duration::from_millis(40);
const OBSERVATION: Duration = Duration::from_secs(1);
const TRIALS: usize = 3;
const SHARED_GROUP: &str = "shared-group";
const IGNORE_HANGUP: &str = "ignore-sighup";
const SEPARATE_GROUP: &str = "separate-group";
const CASES: &[&str] = if cfg!(unix) {
    &[SHARED_GROUP, IGNORE_HANGUP, SEPARATE_GROUP]
} else {
    &[SHARED_GROUP]
};

#[expect(
    clippy::disallowed_methods,
    reason = "fs-write: report fixture startup errors outside the PTY under test"
)]
fn main() -> Result<()> {
    let executable = env::current_exe()?;
    let mut args = env::args_os().skip(1);
    if let (Some(directory), Some(role)) = (args.next(), args.next()) {
        let index: usize = role.to_string_lossy().parse()?;
        let directory = PathBuf::from(directory);
        let layout = args.next().unwrap_or_else(|| SHARED_GROUP.into());
        let result = worker(&executable, &directory, index, &layout);
        if let Err(error) = &result {
            fs::write(directory.join("worker-error"), error.to_string())?;
        }
        return result;
    }

    println!(
        "portable-pty 0.9.0 / {}/{}",
        env::consts::OS,
        env::consts::ARCH
    );
    for &layout in CASES {
        for trial in 1..=TRIALS {
            probe(&executable, layout, trial)?;
        }
    }
    Ok(())
}

fn probe(executable: &Path, layout: &str, trial: usize) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let pty = native_pty_system().openpty(PtySize::default())?;
    let reader = pty.master.try_clone_reader()?;
    let writer = pty.master.take_writer()?;
    let _drain = thread::spawn(move || service_terminal(reader, writer));
    let mut cmd = CommandBuilder::new(executable);
    cmd.cwd(dir.path());
    cmd.arg(dir.path());
    cmd.arg("0");
    cmd.arg(layout);
    let mut root = pty.slave.spawn_command(cmd)?;
    // Keep the terminal open during observation. Closing it can add another signal.
    drop(pty.slave);
    let _master = pty.master;
    let root_pid = root.process_id();

    let ready_until = Instant::now() + Duration::from_secs(5);
    let first = loop {
        match fs::read_to_string(dir.path().join("worker-error")) {
            Ok(error) => return Err(format!("fixture error: {error}").into()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err.into()),
        }
        let counts = heartbeats(dir.path())?;
        if counts.iter().all(|&count| count > 0) {
            break counts;
        }
        if let Some(status) = root.try_wait()? {
            return Err(format!("fixture root exited at startup: {status}; {counts:?}").into());
        }
        if Instant::now() >= ready_until {
            return Err(format!(
                "fixture PID {root_pid:?} did not start all three processes: {counts:?}"
            )
            .into());
        }
        thread::sleep(INTERVAL);
    };
    thread::sleep(OBSERVATION);
    let before = heartbeats(dir.path())?;
    let control = delta(first, before);
    if control.contains(&0) || root.try_wait()?.is_some() {
        return Err(format!("fixture stopped before kill: {control:?}").into());
    }

    root.kill()?;
    let exit_until = Instant::now() + Duration::from_secs(2);
    let exit = loop {
        if let Some(status) = root.try_wait()? {
            break status;
        }
        if Instant::now() >= exit_until {
            return Err("direct child remained alive two seconds after kill".into());
        }
        thread::sleep(INTERVAL);
    };
    let at_exit = heartbeats(dir.path())?;
    thread::sleep(OBSERVATION);
    let after = delta(at_exit, heartbeats(dir.path())?);
    println!(
        "{layout} trial {trial}: control root/child/grandchild {control:?}; root exit {exit}; \
         post-exit root/child/grandchild {after:?}"
    );
    Ok(())
}

fn service_terminal(mut reader: impl io::Read, mut writer: impl Write) -> io::Result<()> {
    if cfg!(windows) {
        // portable-pty 0.9.0 enables PSEUDOCONSOLE_INHERIT_CURSOR. ConPTY
        // waits for a cursor-position reply before its first client can run.
        let request = b"\x1b[6n";
        let mut matched = 0;
        let mut buffer = [0; 1024];
        'handshake: loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            for &byte in &buffer[..read] {
                if byte == request[matched] {
                    matched += 1;
                    if matched == request.len() {
                        writer.write_all(b"\x1b[1;1R")?;
                        break 'handshake;
                    }
                } else {
                    matched = usize::from(byte == request[0]);
                }
            }
        }
    }
    io::copy(&mut reader, &mut io::sink())?;
    Ok(())
}

fn heartbeats(directory: &Path) -> io::Result<[u64; 3]> {
    let mut counts = [0; 3];
    for (role, count) in ROLES.into_iter().zip(&mut counts) {
        match fs::metadata(directory.join(role)) {
            Ok(file) => *count = file.len(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err),
        }
    }
    Ok(counts)
}

fn delta(start: [u64; 3], end: [u64; 3]) -> [u64; 3] {
    std::array::from_fn(|index| end[index] - start[index])
}

#[expect(
    clippy::disallowed_methods,
    reason = "process: spawn the fixture descendants; fs-write: append a private liveness counter"
)]
fn worker(executable: &Path, directory: &Path, index: usize, layout: &OsStr) -> Result<()> {
    let role = ROLES.get(index).ok_or("invalid fixture role")?;
    let heartbeat = directory.join(role);
    File::create_new(&heartbeat)?;
    let mut child = if index + 1 < ROLES.len() {
        let mut command = if cfg!(unix) && index == 0 && layout == IGNORE_HANGUP {
            // Ignored signal dispositions survive exec and are inherited by the
            // grandchild, without changing either descendant's process group.
            let mut shell = Command::new("/bin/sh");
            shell.args(["-c", "trap '' HUP\nexec \"$@\"", "sh"]);
            shell.arg(executable);
            shell
        } else {
            Command::new(executable)
        };
        #[cfg(unix)]
        if index == 0 && layout == SEPARATE_GROUP {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        Some(
            command
                .arg(directory)
                .arg((index + 1).to_string())
                .spawn()?,
        )
    } else {
        None
    };

    // The directory is removed by the driver after observation. The deadline
    // also bounds the life of the workers if the driver itself is terminated.
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match OpenOptions::new().append(true).open(&heartbeat) {
            Ok(mut file) => file.write_all(b".")?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => break,
            Err(err) => return Err(err.into()),
        }
        thread::sleep(INTERVAL);
    }
    if let Some(child) = &mut child {
        child.wait()?;
    }
    Ok(())
}

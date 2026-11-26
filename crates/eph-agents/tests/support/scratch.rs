use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    pub(crate) fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = fs::canonicalize(env::temp_dir())
            .unwrap()
            .join(format!("ephesus-manager-{now}-{id}"));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let Ok(resolved) = fs::canonicalize(&self.0) else {
            return;
        };
        if resolved != self.0 {
            eprintln!("retained redirected headless fixture {}", self.0.display());
            return;
        }
        if let Err(error) = fs::remove_dir_all(resolved) {
            eprintln!("clean headless fixture {}: {error}", self.0.display());
        }
    }
}

#[cfg(unix)]
pub(crate) fn link_dir(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
pub(crate) fn link_dir(target: &Path, link: &Path) {
    use std::time::{Duration, Instant};

    use eph_engines::base_environment;
    use eph_fs::atomic::write_atomic;
    use eph_pty::{ExitStatus, Output, Session, SpawnPlan};

    let parent_path = link.parent().unwrap().to_string_lossy();
    let cwd = Path::new(parent_path.trim_start_matches(r"\\?\"));
    let script = cwd.join("fixture-junction.cmd");
    write_atomic(
        &script,
        b"@echo off\r\nmklink /J \"%FIXTURE_LINK%\" \"%FIXTURE_LINK_TARGET%\"\r\nexit /b %errorlevel%\r\n",
    ).unwrap();
    let inherited: Vec<_> = env::vars_os().collect();
    let mut environment = base_environment(&inherited);
    let target = target.to_string_lossy();
    environment.extend([
        ("FIXTURE_LINK".into(), link.file_name().unwrap().to_owned()),
        ("FIXTURE_LINK_TARGET".into(), target.trim_start_matches(r"\\?\").into()),
    ]);
    let mut session = Session::spawn(SpawnPlan {
        executable: script.as_os_str(),
        args: &[],
        cwd,
        environment: &environment,
        cols: 100,
        rows: 24,
    }, Vec::new()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    loop {
        if let Output::Data(bytes) = session.output(Duration::from_millis(10)).unwrap() {
            output.extend(bytes);
        }
        if let Some(status) = session.try_wait().unwrap() {
            assert_eq!(status, ExitStatus::Code(0), "{}", String::from_utf8_lossy(&output));
            return;
        }
        assert!(Instant::now() < deadline, "mklink did not exit");
    }
}

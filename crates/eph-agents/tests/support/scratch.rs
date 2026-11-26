use std::{
    env, fs,
    path::PathBuf,
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

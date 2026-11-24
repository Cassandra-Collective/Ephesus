//! Engine adapter declarations, spawn plans and probes (FR-1.2, SDD-RUST §3).

mod adapter;
mod config;
mod env;
mod grants;
mod probe;
mod prompt;
mod settings;
mod spec;

pub use adapter::{
    AdapterError, AdapterRegistry, AutonomySupport, Claude, CommitIdentity, ConformanceAdapter,
    EngineAdapter, Environment, HookSupport, Interrupt, NotificationKind, SpawnConfig, SpawnPlan,
    TranscriptReader, UnattendedGrant, UsageFact, assert_autonomy, require_reference_engine,
};
pub use config::{ConfigDir, ConfigError, TrustRecord, Workspace};
pub use grants::{ResolvedTools, ToolGrant, ToolRoot, ToolRoots, resolve_tool_grants};
pub use probe::{ProbeCompletion, ProbeError};
pub use settings::SettingsPlan;
pub use spec::{Authentication, AuthenticationSpec, BinarySpec, CommandSpec};

#[cfg(test)]
mod test_support {
    use std::path::Path;

    #[cfg(unix)]
    pub(crate) fn link_dir(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[cfg(windows)]
    #[expect(
        clippy::disallowed_methods,
        reason = "process: Windows creates the test junction through mklink /J"
    )]
    pub(crate) fn link_dir(target: &Path, link: &Path) {
        let result = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
    }

    pub(crate) fn script(dir: &Path, name: &str, windows: &str, unix: &str) {
        let path = if cfg!(windows) {
            dir.join(format!("{name}.cmd"))
        } else {
            dir.join(name)
        };
        let body = if cfg!(windows) { windows } else { unix };
        eph_fs::atomic::write_atomic(&path, body.as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}

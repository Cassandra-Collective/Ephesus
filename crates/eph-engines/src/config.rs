//! Per-agent configuration, onboarding and workspace trust (ADR-0021/0025/0026).

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use eph_fs::atomic::write_atomic;
use eph_schema::{names::AgentId, registry::EngineId};
use serde_json::{Map, Value};
use thiserror::Error;

/// The engine's private configuration location for one validated hire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDir {
    home: PathBuf,
    path: PathBuf,
}

/// The workspace the Architect approved for this hire (ADR-0021/0025).
#[derive(Debug, Clone, Copy)]
pub enum Workspace<'a> {
    /// A target that must resolve to an existing directory.
    Existing(&'a Path),
    /// An approved worktree whose harness-owned parent already exists.
    FutureWorktree(&'a Path),
}

/// The canonical trust key and whether the approval was already present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRecord {
    /// The forward-slash path recorded in the engine's project map.
    pub project: String,
    /// Whether this call found the consent already written.
    pub already_trusted: bool,
}

/// A private configuration or trust request that could not be applied.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The file could not be accessed or atomically updated.
    #[error("{action} {path}: {source}")]
    Io {
        /// The failed operation.
        action: &'static str,
        /// The path that needs attention.
        path: PathBuf,
        /// The operating system or serialization error.
        source: io::Error,
    },
    /// Leave malformed settings, symbolic links and reparse points in place.
    #[error("refusing {path} ({reason}); repair the private engine configuration before retrying")]
    Refused {
        /// The path this operation refused to use.
        path: PathBuf,
        /// Why it could not be used.
        reason: &'static str,
    },
}

pub(crate) const CLAUDE_CONFIG: &str = ".claude.json";
pub(crate) const CLAUDE_SETTINGS: &str = "eph-settings.json";

impl ConfigDir {
    /// Derive the private directory under the harness home (SDD-RUST §2).
    pub fn for_agent(home: &Path, engine: EngineId, id: &AgentId) -> Result<Self, ConfigError> {
        let home =
            std::path::absolute(home).map_err(|error| at("resolve harness home", home, error))?;
        Ok(Self {
            path: home
                .join("engines")
                .join(engine_name(engine))
                .join(id.as_str()),
            home,
        })
    }

    /// The same path used by the launch, probe and settings plan.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn require_engine(&self, engine: EngineId) -> Result<(), ConfigError> {
        if self.path.parent().and_then(Path::file_name) == Some(engine_name(engine).as_ref()) {
            return Ok(());
        }
        Err(refused(
            &self.path,
            "the private directory belongs to another engine",
        ))
    }

    pub(crate) fn require_agent(&self, id: &AgentId) -> Result<(), ConfigError> {
        if self.path.file_name() == Some(id.as_str().as_ref()) {
            return Ok(());
        }
        Err(refused(
            &self.path,
            "the private directory belongs to another hire",
        ))
    }

    /// Resolve the approved home, then walk only its ordinary subdirectories.
    pub(crate) fn ensure(&self) -> Result<PathBuf, ConfigError> {
        let subtree = self.path.strip_prefix(&self.home).map_err(|_| {
            refused(
                &self.path,
                "the engine directory is outside the harness home",
            )
        })?;
        let mut directory = canonical_dir(&self.home)?;
        for segment in subtree {
            directory.push(segment);
            match fs::create_dir(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(at("create", &directory, error)),
            }
            let metadata = fs::symlink_metadata(&directory)
                .map_err(|error| at("inspect", &directory, error))?;
            if !metadata.file_type().is_dir() || is_link(&metadata) {
                return Err(refused(&directory, "expected a directory without links"));
            }
        }
        Ok(directory)
    }
}

fn engine_name(engine: EngineId) -> &'static str {
    match engine {
        EngineId::Claude => "claude",
        EngineId::Codex => "codex",
        EngineId::Gemini => "gemini",
        EngineId::Grok => "grok",
        EngineId::Opencode => "opencode",
        EngineId::Custom => "custom",
    }
}

pub(crate) fn prepare_claude(config: &ConfigDir) -> Result<Option<PathBuf>, ConfigError> {
    config.require_engine(EngineId::Claude)?;
    let path = config.ensure()?.join(CLAUDE_CONFIG);
    let mut document = read_object(&path)?;
    if document.get("hasCompletedOnboarding") == Some(&Value::Bool(true)) {
        return Ok(None);
    }
    document.insert("hasCompletedOnboarding".into(), Value::Bool(true));
    write_json(&path, &Value::Object(document))?;
    Ok(Some(path))
}

pub(crate) fn trust_claude(
    config: &ConfigDir,
    workspace: Workspace<'_>,
) -> Result<TrustRecord, ConfigError> {
    config.require_engine(EngineId::Claude)?;
    let target = match workspace {
        Workspace::Existing(cwd) => canonical_dir(cwd)?,
        Workspace::FutureWorktree(cwd) => {
            let (Some(parent), Some(Component::Normal(leaf))) =
                (cwd.parent(), cwd.components().next_back())
            else {
                return Err(refused(cwd, "expected a worktree leaf and its parent"));
            };
            canonical_dir(parent)?.join(leaf)
        }
    };
    let project = project_key(&target)?;
    let path = config.ensure()?.join(CLAUDE_CONFIG);
    let mut document = read_object(&path)?;
    let mut projects = take_object(&mut document, "projects");
    let mut entry = take_object(&mut projects, &project);
    let already_trusted = entry.get("hasTrustDialogAccepted") == Some(&Value::Bool(true));
    if !already_trusted {
        entry.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
        projects.insert(project.clone(), Value::Object(entry));
        document.insert("projects".into(), Value::Object(projects));
        write_json(&path, &Value::Object(document))?;
    }
    Ok(TrustRecord {
        project,
        already_trusted,
    })
}

fn take_object(parent: &mut Map<String, Value>, key: &str) -> Map<String, Value> {
    match parent.remove(key) {
        Some(Value::Object(fields)) => fields,
        _ => Map::new(),
    }
}

fn canonical_dir(path: &Path) -> Result<PathBuf, ConfigError> {
    let resolved = fs::canonicalize(path).map_err(|error| at("resolve", path, error))?;
    if !resolved
        .metadata()
        .map_err(|error| at("inspect", path, error))?
        .is_dir()
    {
        return Err(refused(path, "expected an existing directory"));
    }
    Ok(resolved)
}

pub(crate) fn project_key(path: &Path) -> Result<String, ConfigError> {
    let name = path
        .to_str()
        .ok_or_else(|| refused(path, "the engine requires a Unicode path"))?;
    if cfg!(windows) {
        let name = name.replace('\\', "/");
        if let Some(rest) = name.strip_prefix("//?/UNC/") {
            return Ok(format!("//{rest}"));
        }
        return Ok(name.strip_prefix("//?/").unwrap_or(&name).to_owned());
    }
    Ok(name.to_owned())
}

pub(crate) fn read_object(path: &Path) -> Result<Map<String, Value>, ConfigError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(at("inspect", path, error)),
        Ok(metadata) if !metadata.file_type().is_file() || is_link(&metadata) => {
            return Err(refused(
                path,
                "expected a regular JSON file without symlinks or reparse points",
            ));
        }
        Ok(_) => {}
    }
    let bytes = fs::read(path).map_err(|error| at("read", path, error))?;
    serde_json::from_slice(&bytes).map_err(|_| refused(path, "expected a valid JSON object"))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), ConfigError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| at("format", path, io::Error::other(error)))?;
    bytes.push(b'\n');
    write_atomic(path, &bytes).map_err(|error| at("update", path, error))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions as well as ordinary symbolic links.
        const REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

pub(crate) fn at(action: &'static str, path: &Path, source: io::Error) -> ConfigError {
    ConfigError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

pub(crate) fn refused(path: &Path, reason: &'static str) -> ConfigError {
    ConfigError::Refused {
        path: path.to_path_buf(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::link_dir;

    #[test]
    fn a_failed_private_directory_creation_reports_the_create_error() {
        let root = tempfile::tempdir().unwrap();
        let config = ConfigDir {
            home: root.path().to_path_buf(),
            path: root.path().join("bad\0name"),
        };
        assert!(matches!(
            config.ensure(),
            Err(ConfigError::Io { action: "create", source, .. })
                if source.kind() == io::ErrorKind::InvalidInput
        ));
    }

    #[test]
    fn the_private_directory_refuses_a_file_or_a_directory_link() {
        let root = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(root.path()).unwrap();
        let config = ConfigDir {
            home: home.clone(),
            path: home.join("hire"),
        };
        write_atomic(&config.path, b"existing file").unwrap();
        assert!(matches!(
            config.ensure(),
            Err(ConfigError::Refused { path, .. }) if path == config.path
        ));
        assert_eq!(fs::read(&config.path).unwrap(), b"existing file");
        fs::remove_file(&config.path).unwrap();

        let target = root.path().join("other");
        fs::create_dir(&target).unwrap();
        link_dir(&target, &root.path().join("hire"));
        assert!(is_link(&fs::symlink_metadata(&config.path).unwrap()));
        assert!(matches!(
            config.ensure(),
            Err(ConfigError::Refused { path, .. }) if path == config.path
        ));
    }

    #[test]
    fn reading_settings_distinguishes_absence_from_inspection_errors() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            read_object(&root.path().join("missing"))
                .unwrap()
                .is_empty()
        );
        let invalid = root.path().join("bad\0name");
        assert!(matches!(
            read_object(&invalid),
            Err(ConfigError::Io { action: "inspect", path, source })
                if path == invalid && source.kind() == io::ErrorKind::InvalidInput
        ));
        assert!(matches!(
            read_object(root.path()),
            Err(ConfigError::Refused { path, .. }) if path == root.path()
        ));
    }
}

//! Named tool grants. Resolve links before judging containment (ADR-0026).

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Component, Path, PathBuf},
};

use crate::{
    ConfigError,
    config::{at, refused},
};

/// The root a declared plugin path is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRoot {
    /// The activation target.
    Target,
    /// The harness's tools directory.
    Home,
}

/// A relative path to one declared plugin directory.
#[derive(Debug, Clone, Copy)]
pub struct ToolGrant<'a> {
    /// Which approved root the path is relative to.
    pub root: ToolRoot,
    /// Path within that root.
    pub path: &'a Path,
}

/// The two paths underlying the grant vocabulary.
#[derive(Debug, Clone, Copy)]
pub struct ToolRoots<'a> {
    /// The activation target directory.
    pub target: &'a Path,
    /// The harness's tools directory.
    pub home: &'a Path,
}

/// Resolved grants and declarations that point at absent directories.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResolvedTools {
    directories: Vec<PathBuf>,
    missing: Vec<PathBuf>,
}

impl ResolvedTools {
    /// Canonical directories to pass to the engine.
    pub fn directories(&self) -> &[PathBuf] {
        &self.directories
    }

    /// Absent directories to report as degradations.
    pub fn missing(&self) -> &[PathBuf] {
        &self.missing
    }
}

/// Refuse a grant set on escape or damage. Return absent directories to the caller.
pub fn resolve_tool_grants(
    grants: &[ToolGrant<'_>],
    roots: ToolRoots<'_>,
) -> Result<ResolvedTools, ConfigError> {
    let mut result = ResolvedTools::default();
    let mut seen = BTreeSet::new();
    for grant in grants {
        let root = match grant.root {
            ToolRoot::Target => roots.target,
            ToolRoot::Home => roots.home,
        };
        let root = fs::canonicalize(root).map_err(|error| at("resolve tool root", root, error))?;
        if !root.is_dir() {
            return Err(refused(&root, "the tool root must be a directory"));
        }
        let relative = grant.path;
        let mut parts = PathBuf::new();
        if relative.as_os_str().is_empty() {
            return Err(refused(relative, "a tool grant requires a relative path"));
        }
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(part),
                Component::CurDir => {}
                Component::ParentDir if parts.pop() => {}
                _ => {
                    return Err(refused(
                        relative,
                        "a tool grant requires a path inside its root",
                    ));
                }
            }
        }
        let candidate = root.join(parts);
        let mut ancestor = candidate.as_path();
        let directory = loop {
            match fs::canonicalize(ancestor) {
                Ok(resolved) => break resolved,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    confirm_missing_grant(ancestor, fs::symlink_metadata(ancestor))?;
                    ancestor = ancestor.parent().ok_or_else(|| {
                        refused(&candidate, "the tool grant has no resolvable parent")
                    })?;
                }
                Err(error) => return Err(at("resolve tool grant", ancestor, error)),
            }
        };
        if !directory.starts_with(&root) || !directory.is_dir() {
            return Err(refused(
                &candidate,
                "the tool grant escapes its root or is not a directory",
            ));
        }
        if ancestor != candidate {
            result.missing.push(candidate);
        } else if seen.insert(directory.clone()) {
            result.directories.push(directory);
        }
    }
    Ok(result)
}

fn confirm_missing_grant(
    path: &Path,
    inspection: io::Result<fs::Metadata>,
) -> Result<(), ConfigError> {
    match inspection {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(at("inspect tool grant", path, error)),
        Ok(_) => Err(refused(path, "the tool grant is a dangling link")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::link_dir;
    use eph_fs::atomic::write_atomic;

    #[test]
    fn a_missing_resolution_requires_an_actual_absence() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("plugin");
        assert!(confirm_missing_grant(&path, fs::symlink_metadata(&path)).is_ok());
        assert!(matches!(
            confirm_missing_grant(&path, Err(io::ErrorKind::PermissionDenied.into())),
            Err(ConfigError::Io { action: "inspect tool grant", path: refused, source })
                if refused == path && source.kind() == io::ErrorKind::PermissionDenied
        ));

        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        link_dir(&target, &path);
        fs::remove_dir(&target).unwrap();
        assert!(matches!(
            confirm_missing_grant(&path, fs::symlink_metadata(&path)),
            Err(ConfigError::Refused { path: refused, reason: "the tool grant is a dangling link" })
                if refused == path
        ));
        let roots = ToolRoots {
            target: temp.path(),
            home: temp.path(),
        };
        let grant = ToolGrant {
            root: ToolRoot::Home,
            path: Path::new("plugin"),
        };
        let canonical = fs::canonicalize(temp.path()).unwrap().join("plugin");
        assert!(matches!(
            resolve_tool_grants(&[grant], roots),
            Err(ConfigError::Refused { path, reason: "the tool grant is a dangling link" })
                if path == canonical
        ));
    }

    #[test]
    fn grants_refuse_non_directories_and_links_outside_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("approved");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        write_atomic(&root.join("file"), b"file").unwrap();
        link_dir(&outside, &root.join("outside-link"));
        let roots = ToolRoots {
            target: &root,
            home: &root,
        };
        for entry in ["file", "outside-link"] {
            assert!(matches!(
                resolve_tool_grants(&[ToolGrant { root: ToolRoot::Home, path: Path::new(entry) }], roots),
                Err(ConfigError::Refused { path, .. }) if path.ends_with(entry)
            ));
        }
        assert!(matches!(
            resolve_tool_grants(
                &[ToolGrant { root: ToolRoot::Home, path: Path::new("bad\0name") }],
                roots
            ),
            Err(ConfigError::Io { action: "resolve tool grant", source, .. })
                if source.kind() == io::ErrorKind::InvalidInput
        ));
    }
}

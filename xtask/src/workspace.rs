//! The workspace as the checks see it: each member crate, its manifest text, its
//! direct dependencies and its Rust sources.
//!
//! Contract: [`load`] reads the workspace once (`cargo metadata --no-deps
//! --locked`, then each member's `Cargo.toml` and `.rs` files) and returns plain
//! data. It writes nothing. `--locked` matters: an unlocked `cargo metadata` may
//! rewrite a stale `Cargo.lock`, the hole R0.1 closed in the `cargo xtask`
//! alias. The checks take a [`Workspace`] and never touch the disk, so their
//! tests build one in memory.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cargo_metadata::{DependencyKind, MetadataCommand};

/// Which table a dependency is declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DepKind {
    /// `[dependencies]`: shipped with the crate.
    Normal,
    /// `[dev-dependencies]`: tests, examples and benches only.
    Dev,
    /// `[build-dependencies]`: the build script.
    Build,
}

impl DepKind {
    /// The manifest table's name, for messages.
    pub fn table(self) -> &'static str {
        match self {
            DepKind::Normal => "[dependencies]",
            DepKind::Dev => "[dev-dependencies]",
            DepKind::Build => "[build-dependencies]",
        }
    }
}

/// One direct dependency of a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    /// The depended-on package's name (not a `package =` rename's local name).
    pub name: String,
    /// The table it is declared in.
    pub kind: DepKind,
    /// True when it is another member of this workspace.
    pub member: bool,
    /// True when it is a `path =` dependency.
    pub path: bool,
}

/// One Rust source file of a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The file's path.
    pub path: PathBuf,
    /// Its text.
    pub text: String,
}

/// One member crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The package name.
    pub name: String,
    /// Its `Cargo.toml`.
    pub manifest_path: PathBuf,
    /// The `Cargo.toml` text.
    pub manifest: String,
    /// Its direct dependencies, in every table.
    pub deps: Vec<Dep>,
    /// Every `.rs` file under the crate's directory (`target/` and hidden
    /// directories excluded).
    pub sources: Vec<Source>,
}

/// The whole workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Workspace {
    /// Every member, in `cargo metadata`'s order.
    pub members: Vec<Member>,
}

/// Why the workspace could not be read.
#[derive(Debug)]
pub enum LoadError {
    /// `cargo metadata` failed; the payload is its message.
    Metadata(String),
    /// A file could not be read.
    Read(PathBuf, std::io::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Metadata(message) => write!(f, "cargo metadata failed: {message}"),
            LoadError::Read(path, error) => write!(f, "cannot read {}: {error}", path.display()),
        }
    }
}

/// Reads the workspace rooted at `root`. See the module contract.
pub fn load(root: &Path) -> Result<Workspace, LoadError> {
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .other_options(vec!["--locked".to_string()])
        .exec()
        .map_err(|error| LoadError::Metadata(error.to_string()))?;
    let names: BTreeSet<String> = metadata
        .workspace_packages()
        .iter()
        .map(|package| package.name.to_string())
        .collect();
    let mut members = Vec::new();
    for package in metadata.workspace_packages() {
        let manifest_path = package.manifest_path.clone().into_std_path_buf();
        let manifest = read(&manifest_path)?;
        let deps = package
            .dependencies
            .iter()
            .map(|dep| Dep {
                name: dep.name.clone(),
                kind: match dep.kind {
                    DependencyKind::Development => DepKind::Dev,
                    DependencyKind::Build => DepKind::Build,
                    _ => DepKind::Normal,
                },
                member: dep.path.is_some() && names.contains(&dep.name),
                path: dep.path.is_some(),
            })
            .collect();
        let dir = manifest_path.parent().unwrap_or(root).to_path_buf();
        let mut paths = Vec::new();
        rust_files(&dir, &mut paths)?;
        paths.sort();
        let sources = paths
            .into_iter()
            .map(|path| read(&path).map(|text| Source { path, text }))
            .collect::<Result<Vec<_>, _>>()?;
        members.push(Member {
            name: package.name.to_string(),
            manifest_path,
            manifest,
            deps,
            sources,
        });
    }
    Ok(Workspace { members })
}

fn read(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path).map_err(|error| LoadError::Read(path.to_path_buf(), error))
}

/// Whether the source walk skips a subdirectory: `target/` (build output),
/// a hidden directory, or one holding its own `Cargo.toml`, which is another
/// package (a test fixture workspace, say) whose sources are not this crate's.
/// A pure predicate so the rule is testable in a tree cargo-mutants copied,
/// where no `target/` directory exists to walk past.
pub fn skips_dir(name: &str, holds_manifest: bool) -> bool {
    name == "target" || name.starts_with('.') || holds_manifest
}

/// Every `.rs` file under `dir`, skipping what [`skips_dir`] names.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), LoadError> {
    let entries =
        std::fs::read_dir(dir).map_err(|error| LoadError::Read(dir.to_path_buf(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| LoadError::Read(dir.to_path_buf(), error))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry
            .file_type()
            .map_err(|error| LoadError::Read(path.clone(), error))?;
        if kind.is_dir() {
            if !skips_dir(&name, path.join("Cargo.toml").is_file()) {
                rust_files(&path, out)?;
            }
        } else if kind.is_file() && name.ends_with(".rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    fn dep<'a>(member: &'a Member, name: &str) -> &'a Dep {
        member.deps.iter().find(|dep| dep.name == name).unwrap()
    }

    #[test]
    fn a_fixture_workspace_loads_with_every_dependency_kind() {
        let workspace = load(&fixture("clean")).unwrap();
        assert_eq!(workspace.members.len(), 1);
        let agora = &workspace.members[0];
        assert_eq!(agora.name, "eph-agora");
        assert!(agora.manifest.contains("[lints]"));
        assert_eq!(dep(agora, "serde").kind, DepKind::Normal);
        assert_eq!(dep(agora, "tempfile").kind, DepKind::Dev);
        assert_eq!(dep(agora, "syn").kind, DepKind::Build);
        assert!(agora.deps.iter().all(|dep| !dep.member && !dep.path));
    }

    #[test]
    fn target_hidden_and_nested_package_directories_are_not_scanned() {
        let workspace = load(&fixture("clean")).unwrap();
        let names: Vec<String> = workspace.members[0]
            .sources
            .iter()
            .map(|source| {
                source
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        // target/junk.rs and .hidden/junk.rs exist and break every rule.
        assert_eq!(names, ["lib.rs"]);
        // And this crate's own fixtures, each a package of its own, are not
        // read as xtask's sources.
        let here = load(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap();
        let xtask = here.members.iter().find(|m| m.name == "xtask").unwrap();
        assert!(!xtask.sources.is_empty());
        assert!(
            xtask
                .sources
                .iter()
                .all(|source| !source.path.to_string_lossy().contains("fixtures")),
            "a fixture source was read as xtask's"
        );
    }

    #[test]
    fn a_path_dependency_outside_the_workspace_is_a_path_but_not_a_member() {
        let workspace = load(&fixture("violating")).unwrap();
        let ui = &workspace.members[0];
        let outside = dep(ui, "outside");
        assert!(outside.path);
        assert!(!outside.member);
        let registry = dep(ui, "left-pad");
        assert!(!registry.path && !registry.member);
    }

    #[test]
    fn load_errors_say_what_failed() {
        let missing = fixture("no-such-workspace");
        let error = load(&missing).unwrap_err();
        assert!(
            error.to_string().starts_with("cargo metadata failed: "),
            "{error}"
        );
        let read = LoadError::Read(
            missing.clone(),
            std::io::Error::new(std::io::ErrorKind::NotFound, "gone"),
        );
        assert_eq!(
            read.to_string(),
            format!("cannot read {}: gone", missing.display())
        );
    }

    #[test]
    fn build_output_hidden_directories_and_other_packages_are_skipped() {
        assert!(skips_dir("target", false));
        assert!(skips_dir(".git", false));
        assert!(skips_dir(".hidden", false));
        assert!(skips_dir("fixtures", true));
        assert!(!skips_dir("src", false));
        assert!(!skips_dir("targets", false));
        assert!(!skips_dir("tests", false));
    }

    #[test]
    fn each_dependency_table_is_named_for_messages() {
        assert_eq!(DepKind::Normal.table(), "[dependencies]");
        assert_eq!(DepKind::Dev.table(), "[dev-dependencies]");
        assert_eq!(DepKind::Build.table(), "[build-dependencies]");
    }
}

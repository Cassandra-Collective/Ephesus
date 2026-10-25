//! Invariant 10 by machine: every direct dependency is approved
//! (docs/RUST-PLAN.md §4 item 6).
//!
//! Contract: [`check`] returns a violation for every direct dependency, in any
//! table, of any member, that is neither another member of this workspace nor a
//! crate in [`APPROVED`]. A `path` dependency outside the workspace is refused
//! too, because no memo can approve code that has no registry identity. It
//! reads nothing and writes nothing. [`APPROVED`] is the machine copy of the
//! memo, `docs/RUST-CRATES.md`; a test pins the two to each other, so a crate
//! cannot be approved in one and not the other.

use crate::invariants::{Violation, manifest_line};
use crate::workspace::Workspace;

/// Every crate an approved set in docs/RUST-CRATES.md lists as a dependency
/// (Set A, approved 2026-10-19, plus the rows approved by their own memo). The
/// cargo tools there (`cargo-deny`, `cargo-llvm-cov`, `cargo-mutants`) are
/// installed binaries, not dependencies, and are not listed.
pub const APPROVED: &[&str] = &[
    "serde",
    "serde_json",
    "thiserror",
    "jiff",
    "uuid",
    "tokio",
    "portable-pty",
    "notify",
    "rusqlite",
    "egui",
    "eframe",
    "alacritty_terminal",
    "tempfile",
    "egui_kittest",
    "syn",
    "cargo_metadata",
    "toml",
    "proc-macro2",
];

/// Every unapproved direct dependency in `workspace`.
pub fn check(workspace: &Workspace) -> Vec<Violation> {
    let mut out = Vec::new();
    for member in &workspace.members {
        for dep in &member.deps {
            if dep.member {
                continue;
            }
            let message = if dep.path {
                format!(
                    "`{}` depends on `{}` by path, and it is not a member of this workspace. \
                     Make it a member, or depend on an approved registry crate (invariant 10)",
                    member.name, dep.name
                )
            } else if APPROVED.contains(&dep.name.as_str()) {
                continue;
            } else {
                format!(
                    "`{}` depends on `{}` in {}, and no approved set in docs/RUST-CRATES.md lists it. \
                     Invariant 10: write its memo row and get the Architect's approval before adding it",
                    member.name,
                    dep.name,
                    dep.kind.table()
                )
            };
            out.push(Violation {
                rule: "deps",
                file: member.manifest_path.clone(),
                line: manifest_line(&member.manifest, &dep.name),
                message,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::*;
    use crate::workspace::{Dep, DepKind, Member};

    fn member(deps: Vec<Dep>) -> Member {
        Member {
            name: "eph-agora".to_string(),
            manifest_path: PathBuf::from("crates/eph-agora/Cargo.toml"),
            manifest: "[package]\nname = \"eph-agora\"\n\n[dependencies]\nleft-pad = \"1\"\nvendored = { path = \"../../vendor/vendored\" }\n".to_string(),
            deps,
            sources: Vec::new(),
        }
    }

    fn dep(name: &str, kind: DepKind, member: bool, path: bool) -> Dep {
        Dep {
            name: name.to_string(),
            kind,
            member,
            path,
        }
    }

    #[test]
    fn approved_crates_and_members_pass() {
        let workspace = Workspace {
            members: vec![member(vec![
                dep("serde", DepKind::Normal, false, false),
                dep("tempfile", DepKind::Dev, false, false),
                dep("eph-fs", DepKind::Normal, true, true),
            ])],
        };
        assert_eq!(check(&workspace), vec![]);
    }

    #[test]
    fn an_unapproved_crate_fails_in_every_table_naming_the_memo() {
        for kind in [DepKind::Normal, DepKind::Dev, DepKind::Build] {
            let workspace = Workspace {
                members: vec![member(vec![dep("left-pad", kind, false, false)])],
            };
            let violations = check(&workspace);
            assert_eq!(violations.len(), 1, "{kind:?}");
            assert_eq!(violations[0].rule, "deps");
            assert_eq!(violations[0].line, 5);
            assert!(violations[0].message.contains("docs/RUST-CRATES.md"));
            assert!(violations[0].message.contains(kind.table()));
        }
    }

    #[test]
    fn a_path_dependency_outside_the_workspace_fails_even_with_an_approved_name() {
        // Named like an approved crate on purpose: a path is not the registry
        // crate the memo approved.
        let workspace = Workspace {
            members: vec![member(vec![dep("serde", DepKind::Normal, false, true)])],
        };
        let violations = check(&workspace);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].message.contains("by path"));
    }

    /// The crate names in RUST-CRATES.md's Set A tables and memo rows, without
    /// the "Cargo tools" table (installed binaries) and without Set B.
    fn memo_crates() -> BTreeSet<String> {
        let path = crate::gate::workspace_root().join("docs/RUST-CRATES.md");
        let memo = std::fs::read_to_string(&path).unwrap();
        let set_a = memo
            .split("## Set A")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .unwrap();
        let mut names = BTreeSet::new();
        let mut in_tools = false;
        for line in set_a.lines() {
            if line.starts_with("### ") {
                in_tools = line.contains("Cargo tools");
                continue;
            }
            if in_tools || !line.starts_with("| `") {
                continue;
            }
            let name = line.split('`').nth(1).unwrap();
            names.insert(name.to_string());
        }
        names
    }

    #[test]
    fn the_approved_list_is_exactly_the_memo() {
        let memo = memo_crates();
        let code: BTreeSet<String> = APPROVED.iter().map(|name| (*name).to_string()).collect();
        assert_eq!(
            code, memo,
            "xtask's APPROVED and docs/RUST-CRATES.md disagree; a crate is approved in both or in neither"
        );
        assert_eq!(APPROVED.len(), code.len(), "APPROVED lists a crate twice");
    }
}

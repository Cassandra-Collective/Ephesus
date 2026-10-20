//! `cargo xtask` — the Ephesus workspace's own tooling (docs/RUST-PLAN.md §4).
//!
//! `cargo xtask gate` runs the gate chain in order and stops at the first red
//! step. `invariants` and `deps` are two of its steps and run on their own too.
//! `mutants <package>` runs a mutation round with its control. The binary is a
//! thin shell over [`dispatch`], so the tests drive the same code the command
//! line does, and every exit code is a plain number they can assert.

pub mod deps;
pub mod gate;
pub mod invariants;
pub mod mutants;
pub mod workspace;

use std::path::Path;

use gate::Check;

/// What `cargo xtask help` prints.
pub const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  gate                run the gate chain (fmt, clippy, test, deny, invariants, deps)
                      and stop at the first red step
  invariants          check the invariant rules over the workspace (RUST-PLAN §4 item 5)
  deps                check every direct dependency is approved (invariant 10)
  mutants <package>   run a mutation round and its control (GYM-008's rules)
  help                print this message
";

/// Runs the command named by `args` (the arguments after the program name) and
/// returns its exit code: 0 on success, 1 when a check, the gate or a mutation
/// round is red, 2 for an unknown command or a missing argument, 3 for an
/// INVALID mutation round.
pub fn dispatch(args: &[String]) -> u8 {
    match args.first().map(String::as_str) {
        Some("gate") => gate::main(),
        Some("invariants") => run_check(&gate::workspace_root(), Check::Invariants),
        Some("deps") => run_check(&gate::workspace_root(), Check::Deps),
        Some("mutants") => mutants::main(&args[1..]),
        Some("help" | "--help" | "-h") | None => {
            print!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("xtask: unknown command `{other}`\n\n{USAGE}");
            2
        }
    }
}

/// Whether a check read too little to prove anything: no crate, or no source
/// file. Memory: a run with zero tests is invalid, not a pass.
pub fn read_nothing(crates: usize, files: usize) -> bool {
    crates == 0 || files == 0
}

/// Runs one check over the workspace at `root`, printing every violation and a
/// summary that says how much it read. Returns 0 when it found none, else 1. A
/// check that read nothing fails ([`read_nothing`]).
pub fn run_check(root: &Path, check: Check) -> u8 {
    let name = match check {
        Check::Invariants => "invariants",
        Check::Deps => "deps",
    };
    let workspace = match workspace::load(root) {
        Ok(workspace) => workspace,
        Err(error) => {
            println!("{name}: cannot read the workspace: {error}");
            return 1;
        }
    };
    let crates = workspace.members.len();
    let files: usize = workspace.members.iter().map(|m| m.sources.len()).sum();
    if read_nothing(crates, files) {
        println!(
            "{name}: read {crates} crate(s) and {files} source file(s); a check over nothing proves nothing"
        );
        return 1;
    }
    let violations = match check {
        Check::Invariants => invariants::check(&workspace),
        Check::Deps => deps::check(&workspace),
    };
    for violation in &violations {
        println!("{name}: {violation}");
    }
    if violations.is_empty() {
        println!("{name}: ok ({crates} crate(s), {files} source file(s))");
        0
    } else {
        println!("{name}: {} violation(s)", violations.len());
        1
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn a_check_over_no_crate_or_no_file_proves_nothing() {
        assert!(read_nothing(0, 0));
        assert!(read_nothing(0, 3));
        assert!(read_nothing(1, 0));
        assert!(!read_nothing(1, 1));
    }

    #[test]
    fn each_check_passes_a_clean_workspace_and_fails_a_violating_one() {
        for check in [Check::Invariants, Check::Deps] {
            assert_eq!(run_check(&fixture("clean"), check), 0, "{check:?} on clean");
            assert_eq!(
                run_check(&fixture("violating"), check),
                1,
                "{check:?} on violating"
            );
        }
    }

    #[test]
    fn an_unreadable_workspace_fails_the_check() {
        assert_eq!(
            run_check(&fixture("no-such-workspace"), Check::Invariants),
            1
        );
    }

    #[test]
    fn dispatch_returns_each_documented_exit_code() {
        let args = |list: &[&str]| list.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();
        assert_eq!(dispatch(&args(&["help"])), 0);
        assert_eq!(dispatch(&args(&[])), 0);
        assert_eq!(dispatch(&args(&["bogus"])), 2);
        assert_eq!(dispatch(&args(&["mutants"])), 2);
        assert_eq!(dispatch(&args(&["invariants"])), 0);
        assert_eq!(dispatch(&args(&["deps"])), 0);
    }
}

//! `cargo xtask` — the Ephesus workspace's own tooling (docs/RUST-PLAN.md §4).
//!
//! `cargo xtask gate` runs the gate chain in order and stops at the first red
//! step. R0.2 adds the `invariants` and `deps` checks to the chain. The binary
//! is a thin shell over [`dispatch`], so the integration tests drive the same
//! code the command line does.

pub mod gate;

use std::process::ExitCode;

/// What `cargo xtask help` prints.
pub const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  gate    run the gate chain (fmt, clippy, test, deny) and stop at the first red step
  help    print this message
";

/// Runs the command named by `args` (the arguments after the program name).
/// Exit codes: 0 on success, 1 when the gate is red, 2 for an unknown command.
pub fn dispatch(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("gate") => gate::main(),
        Some("help" | "--help" | "-h") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("xtask: unknown command `{other}`\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

//! The xtask binary and its real cargo runner, driven from outside.
//!
//! The unit tests in `gate.rs` replace the runner with a script, which leaves
//! the real runner and both entry points untested (memory: a seam test does not
//! test the resolver). Here the xtask binary stands in for cargo: pointed at
//! itself through `CARGO`, the gate's first step runs `xtask fmt --all --check`,
//! an unknown command, which exits 2, so the real chain must report it red.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use xtask::gate::{CargoRunner, Runner, workspace_root};

const XTASK: &str = env!("CARGO_BIN_EXE_xtask");

#[expect(
    clippy::disallowed_methods,
    reason = "the test runs the xtask binary it is testing"
)]
fn xtask(args: &[&str]) -> Command {
    let mut command = Command::new(XTASK);
    command.args(args);
    command
}

#[test]
fn help_and_no_arguments_print_usage_and_succeed() {
    for args in [&["help"][..], &[]] {
        let out = xtask(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "args {args:?}");
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("usage: cargo xtask"));
    }
}

#[test]
fn an_unknown_command_exits_2_and_names_it() {
    let out = xtask(&["bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown command `bogus`"));
}

#[test]
fn the_real_gate_reports_a_failing_cargo_red_and_exits_1() {
    let out = xtask(&["gate"]).env("CARGO", XTASK).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "stdout: {stdout}");
    assert!(
        stdout.contains("gate: fmt FAILED (cargo exited 2)"),
        "{stdout}"
    );
    assert!(!stdout.contains("gate: fmt ok"), "{stdout}");
    assert!(stdout.ends_with("gate: RED\n"), "{stdout}");
}

#[test]
fn the_real_runner_returns_the_exit_code_or_none() {
    let root = workspace_root();
    let mut runner = CargoRunner {
        cargo: OsString::from(XTASK),
    };
    assert_eq!(runner.cargo(&root, &["help"]), Some(0));
    assert_eq!(runner.cargo(&root, &["bogus"]), Some(2));
    assert_eq!(runner.cargo_silent(&root, &["help"]), Some(0));
    assert_eq!(runner.cargo_silent(&root, &["bogus"]), Some(2));
    let mut missing = CargoRunner {
        cargo: OsString::from("ephesus-no-such-cargo-binary"),
    };
    assert_eq!(missing.cargo(&root, &["help"]), None);
    assert_eq!(missing.cargo_silent(&root, &["help"]), None);
}

#[test]
fn the_workspace_root_is_the_directory_holding_the_workspace_manifest() {
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[workspace]"));
    assert!(Path::new(&root).join("xtask").is_dir());
}

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

use xtask::coverage::{Harness, Real};
use xtask::gate::{CargoRunner, Check, Runner, workspace_root};

const XTASK: &str = env!("CARGO_BIN_EXE_xtask");

#[expect(
    clippy::disallowed_methods,
    reason = "process: the test runs the xtask binary it is testing"
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
fn invariants_and_deps_pass_over_this_workspace_and_say_what_they_read() {
    for check in ["invariants", "deps"] {
        let out = xtask(&[check]).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{check}: {stdout}");
        assert!(stdout.starts_with(&format!("{check}: ok (")), "{stdout}");
        assert!(stdout.contains("source file(s))"), "{stdout}");
    }
}

#[test]
fn mutants_without_a_package_is_a_usage_error() {
    let out = xtask(&["mutants"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: cargo xtask mutants <package>"));
}

#[test]
fn the_workspace_root_is_the_directory_holding_the_workspace_manifest() {
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[workspace]"));
    assert!(Path::new(&root).join("xtask").is_dir());
}

#[test]
fn the_real_runner_runs_each_check_and_returns_its_verdict() {
    let fixtures = workspace_root()
        .join("xtask")
        .join("tests")
        .join("fixtures");
    let mut runner = CargoRunner {
        cargo: OsString::from(XTASK),
    };
    for check in [Check::Invariants, Check::Deps] {
        assert_eq!(
            runner.check(&fixtures.join("clean"), check),
            Some(0),
            "{check:?}"
        );
        assert_eq!(
            runner.check(&fixtures.join("violating"), check),
            Some(1),
            "{check:?}"
        );
    }
}

#[test]
fn coverage_with_unknown_arguments_is_a_usage_error() {
    for args in [
        &["coverage", "bogus"][..],
        &["coverage", "ratchet"],
        &["coverage", "emit", "x"],
    ] {
        let out = xtask(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("usage: cargo xtask coverage [emit | ratchet <dir>]"),
            "{args:?}"
        );
    }
}

#[test]
fn a_ratchet_over_no_measurement_says_what_is_missing_and_prints_no_floors() {
    let dir = std::env::temp_dir().join(format!("ephesus-ratchet-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = xtask(&["coverage", "ratchet", dir.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    for os in ["linux", "macos", "windows"] {
        assert!(
            stderr.contains(&format!("coverage ratchet: {os}: 0 of the 3 CI runs")),
            "{stderr}"
        );
    }
    assert!(
        out.stdout.is_empty(),
        "a refused ratchet prints nothing to redirect"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_real_coverage_harness_returns_cargos_exit_code_or_none() {
    let root = workspace_root();
    let report = std::env::temp_dir()
        .join("ephesus-no-report")
        .join("lcov.info");
    // The xtask binary stands in for cargo: `xtask llvm-cov ...` is an
    // unknown command, which exits 2.
    let mut stand_in = Real {
        cargo: OsString::from(XTASK),
    };
    assert_eq!(stand_in.tool(), None, "a failing --version is no tool");
    assert_eq!(stand_in.llvm_cov(&root, &report), Some(2));
    let mut missing = Real {
        cargo: OsString::from("ephesus-no-such-cargo-binary"),
    };
    assert_eq!(missing.tool(), None);
    assert_eq!(missing.llvm_cov(&root, &report), None);
}

#[test]
fn the_real_gate_runner_reports_a_coverage_step_without_its_tool_red() {
    // The gate's coverage step through the real runner, with the xtask binary
    // as cargo: its probe fails, so the step says how to install the tool and
    // exits 1. A green coverage step needs a real instrumented run of a whole
    // workspace, which `cargo xtask gate` performs on every invocation; a
    // test of it here would nest one coverage run inside another (xtask
    // mutation round 1 triage, 2026-10-20).
    let mut runner = CargoRunner {
        cargo: OsString::from(XTASK),
    };
    assert_eq!(runner.coverage(&workspace_root()), Some(1));
}

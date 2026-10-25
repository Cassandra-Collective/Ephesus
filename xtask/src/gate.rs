//! The gate chain (docs/RUST-PLAN.md §4).
//!
//! Contract: [`run`] executes [`STEPS`] in order through a [`Runner`] and stops
//! at the first step that does not pass. A step whose tool is not installed
//! fails with the command that installs it, because a refusal must teach the
//! rule rather than only state it. This module writes nothing. Its side effects
//! are the cargo processes the runner starts and the files xtask's own checks
//! read.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// A cargo subcommand a step needs that does not ship with the toolchain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    /// Arguments to `cargo` that succeed only when the tool is installed.
    pub probe: &'static [&'static str],
    /// The command that installs the approved version (docs/RUST-CRATES.md).
    pub install: &'static str,
}

/// One of xtask's own checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// `cargo xtask invariants` (RUST-PLAN §4 item 5).
    Invariants,
    /// `cargo xtask deps` (RUST-PLAN §4 item 6).
    Deps,
}

/// What a step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `cargo <args>`.
    Cargo(&'static [&'static str]),
    /// One of xtask's own checks, run in this process.
    Check(Check),
}

/// One step of the chain: what it runs and what it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The name printed in the report.
    pub name: &'static str,
    /// What it runs.
    pub action: Action,
    /// The tool the step needs beyond cargo itself, if any.
    pub requires: Option<Tool>,
}

/// The chain, in the order RUST-PLAN §4 fixes. `--locked` makes a stale
/// `Cargo.lock` fail the gate instead of being rewritten in silence.
pub const STEPS: &[Step] = &[
    Step {
        name: "fmt",
        action: Action::Cargo(&["fmt", "--all", "--check"]),
        requires: None,
    },
    Step {
        name: "clippy",
        action: Action::Cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
        requires: None,
    },
    Step {
        name: "test",
        action: Action::Cargo(&["test", "--workspace", "--locked"]),
        requires: None,
    },
    Step {
        name: "deny",
        action: Action::Cargo(&["deny", "check"]),
        requires: Some(Tool {
            probe: &["deny", "--version"],
            install: "cargo install cargo-deny --version 0.20.2 --locked",
        }),
    },
    Step {
        name: "invariants",
        action: Action::Check(Check::Invariants),
        requires: None,
    },
    Step {
        name: "deps",
        action: Action::Check(Check::Deps),
        requires: None,
    },
];

/// How a step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    /// Cargo exited 0.
    Passed,
    /// Cargo exited with this non-zero code.
    Failed(i32),
    /// Cargo could not be started, or was ended by a signal: no exit code.
    DidNotFinish,
    /// The step's tool is not installed; the payload is its install command.
    ToolMissing(&'static str),
}

/// Starts cargo. Abstracted so the chain's ordering and stopping rules are
/// testable without running cargo.
pub trait Runner {
    /// Runs `cargo <args>` in `dir` with inherited stdio. Returns the exit
    /// code, or `None` if cargo could not start or ended without one.
    fn cargo(&mut self, dir: &Path, args: &[&str]) -> Option<i32>;
    /// Runs `cargo <args>` in `dir` with all output discarded, for probes.
    fn cargo_silent(&mut self, dir: &Path, args: &[&str]) -> Option<i32>;
    /// Runs one of xtask's own checks over the workspace at `root`, printing
    /// its findings. Returns its exit code.
    fn check(&mut self, root: &Path, check: Check) -> Option<i32>;
}

/// Runs the chain in order and stops at the first step that does not pass.
/// Returns every step attempted with its result; the last is the failure,
/// if any.
pub fn run(runner: &mut dyn Runner, root: &Path) -> Vec<(&'static str, StepResult)> {
    let mut report = Vec::new();
    for step in STEPS {
        let result = run_step(runner, root, step);
        let passed = result == StepResult::Passed;
        report.push((step.name, result));
        if !passed {
            break;
        }
    }
    report
}

fn run_step(runner: &mut dyn Runner, root: &Path, step: &Step) -> StepResult {
    if let Some(tool) = step.requires
        && runner.cargo_silent(root, tool.probe) != Some(0)
    {
        return StepResult::ToolMissing(tool.install);
    }
    let code = match step.action {
        Action::Cargo(args) => runner.cargo(root, args),
        Action::Check(check) => runner.check(root, check),
    };
    match code {
        Some(0) => StepResult::Passed,
        Some(code) => StepResult::Failed(code),
        None => StepResult::DidNotFinish,
    }
}

/// One line per attempted step, and whether the chain as a whole passed.
pub fn render(report: &[(&'static str, StepResult)]) -> (String, bool) {
    let mut out = String::new();
    let mut green = report.len() == STEPS.len();
    for (name, result) in report {
        let line = match result {
            StepResult::Passed => format!("gate: {name} ok\n"),
            StepResult::Failed(code) => {
                green = false;
                let own_check = STEPS
                    .iter()
                    .any(|step| step.name == *name && matches!(step.action, Action::Check(_)));
                if own_check {
                    format!("gate: {name} FAILED (its findings are above); the chain stops here\n")
                } else {
                    format!("gate: {name} FAILED (cargo exited {code}); the chain stops here\n")
                }
            }
            StepResult::DidNotFinish => {
                green = false;
                format!(
                    "gate: {name} FAILED (cargo did not start or ended without an exit code); the chain stops here\n"
                )
            }
            StepResult::ToolMissing(install) => {
                green = false;
                format!(
                    "gate: {name} FAILED: its tool is not installed. Install the approved version (docs/RUST-CRATES.md):\n    {install}\n"
                )
            }
        };
        out.push_str(&line);
    }
    out.push_str(if green {
        "gate: all steps green\n"
    } else {
        "gate: RED\n"
    });
    (out, green)
}

/// The warning owed when cargo would write its build output inside OneDrive:
/// slow, and fought over by the sync client (RUST-PLAN §8). `None` when
/// `CARGO_TARGET_DIR` is set, or when the workspace is not under OneDrive.
pub fn target_dir_warning(root: &Path, target_dir: Option<&OsString>) -> Option<String> {
    if target_dir.is_some_and(|dir| !dir.is_empty()) {
        return None;
    }
    let under_onedrive = root.components().any(|part| match part {
        Component::Normal(name) => name.to_string_lossy().starts_with("OneDrive"),
        _ => false,
    });
    under_onedrive.then(|| {
        format!(
            "gate: warning: CARGO_TARGET_DIR is not set, so cargo builds into {} inside OneDrive.\n    \
             Set it to a directory outside OneDrive (RUST-PLAN R0.1), for example\n    \
             %LOCALAPPDATA%\\ephesus-cargo-target on Windows.\n",
            root.join("target").display()
        )
    })
}

/// The real runner: starts the cargo binary it holds.
pub struct CargoRunner {
    /// The cargo to run: the one that launched xtask (`CARGO`), or `cargo`.
    pub cargo: OsString,
}

impl CargoRunner {
    #[expect(
        clippy::disallowed_methods,
        reason = "process: RUST-PLAN §3.2 allowlists xtask; running cargo is the gate chain's whole job"
    )]
    fn command(&self, dir: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(&self.cargo);
        command.args(args).current_dir(dir);
        command
    }
}

impl Runner for CargoRunner {
    fn cargo(&mut self, dir: &Path, args: &[&str]) -> Option<i32> {
        self.command(dir, args).status().ok()?.code()
    }

    fn cargo_silent(&mut self, dir: &Path, args: &[&str]) -> Option<i32> {
        self.command(dir, args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?
            .code()
    }

    fn check(&mut self, root: &Path, check: Check) -> Option<i32> {
        Some(i32::from(crate::run_check(root, check)))
    }
}

/// The workspace root: xtask's own manifest directory's parent.
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

/// `cargo xtask gate`.
pub fn main() -> u8 {
    let root = workspace_root();
    if let Some(warning) = target_dir_warning(&root, std::env::var_os("CARGO_TARGET_DIR").as_ref())
    {
        eprint!("{warning}");
    }
    let mut runner = CargoRunner {
        cargo: std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
    };
    let (text, green) = render(&run(&mut runner, &root));
    print!("{text}");
    u8::from(!green)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers each `cargo` call from a script and records every call.
    struct Scripted {
        /// Exit code per step name; a step absent here passes.
        codes: Vec<(&'static str, Option<i32>)>,
        /// Probe result for tools; `Some(0)` means installed.
        probe: Option<i32>,
        calls: Vec<String>,
    }

    impl Scripted {
        fn new(codes: Vec<(&'static str, Option<i32>)>, probe: Option<i32>) -> Self {
            Scripted {
                codes,
                probe,
                calls: Vec::new(),
            }
        }
    }

    impl Runner for Scripted {
        fn cargo(&mut self, _dir: &Path, args: &[&str]) -> Option<i32> {
            let name = args[0];
            self.calls.push(name.to_string());
            self.codes
                .iter()
                .find(|(step, _)| *step == name)
                .map_or(Some(0), |(_, code)| *code)
        }

        fn cargo_silent(&mut self, _dir: &Path, args: &[&str]) -> Option<i32> {
            self.calls.push(format!("probe:{}", args.join(" ")));
            self.probe
        }

        fn check(&mut self, _root: &Path, check: Check) -> Option<i32> {
            let name = match check {
                Check::Invariants => "invariants",
                Check::Deps => "deps",
            };
            self.calls.push(format!("check:{name}"));
            self.codes
                .iter()
                .find(|(step, _)| *step == name)
                .map_or(Some(0), |(_, code)| *code)
        }
    }

    #[test]
    fn the_chain_is_fmt_clippy_test_deny_in_that_order() {
        let names: Vec<&str> = STEPS.iter().map(|step| step.name).collect();
        assert_eq!(
            names,
            ["fmt", "clippy", "test", "deny", "invariants", "deps"]
        );
    }

    #[test]
    fn clippy_denies_warnings_and_every_build_step_is_locked() {
        let args = |name: &str| match STEPS.iter().find(|s| s.name == name).unwrap().action {
            Action::Cargo(args) => args,
            Action::Check(_) => panic!("{name} is not a cargo step"),
        };
        assert!(args("clippy").ends_with(&["--", "-D", "warnings"]));
        for name in ["clippy", "test"] {
            assert!(args(name).contains(&"--locked"), "{name} must be --locked");
        }
    }

    #[test]
    fn a_green_chain_runs_every_step_and_passes() {
        let mut runner = Scripted::new(vec![], Some(0));
        let report = run(&mut runner, Path::new("."));
        assert_eq!(
            runner.calls,
            [
                "fmt",
                "clippy",
                "test",
                "probe:deny --version",
                "deny",
                "check:invariants",
                "check:deps"
            ]
        );
        let (text, green) = render(&report);
        assert!(green);
        assert!(text.ends_with("gate: all steps green\n"));
    }

    #[test]
    fn the_chain_stops_at_the_first_red_step() {
        let mut runner = Scripted::new(vec![("clippy", Some(101))], Some(0));
        let report = run(&mut runner, Path::new("."));
        assert_eq!(runner.calls, ["fmt", "clippy"]);
        assert_eq!(report.last(), Some(&("clippy", StepResult::Failed(101))));
        let (text, green) = render(&report);
        assert!(!green);
        assert!(text.contains("gate: clippy FAILED (cargo exited 101)"));
        assert!(text.ends_with("gate: RED\n"));
    }

    #[test]
    fn a_failing_check_stops_the_chain_and_points_at_its_findings() {
        let mut runner = Scripted::new(vec![("invariants", Some(1))], Some(0));
        let report = run(&mut runner, Path::new("."));
        assert_eq!(
            runner.calls.last().map(String::as_str),
            Some("check:invariants")
        );
        assert_eq!(report.last(), Some(&("invariants", StepResult::Failed(1))));
        let (text, green) = render(&report);
        assert!(!green);
        assert!(text.contains("gate: invariants FAILED (its findings are above)"));
    }

    #[test]
    fn a_step_that_does_not_finish_is_red_not_green() {
        let mut runner = Scripted::new(vec![("test", None)], Some(0));
        let report = run(&mut runner, Path::new("."));
        assert_eq!(report.last(), Some(&("test", StepResult::DidNotFinish)));
        assert!(!render(&report).1);
    }

    #[test]
    fn a_missing_tool_fails_with_the_command_that_installs_it() {
        let mut runner = Scripted::new(vec![], Some(101));
        let report = run(&mut runner, Path::new("."));
        // The tool is probed and its step never runs.
        assert_eq!(
            runner.calls,
            ["fmt", "clippy", "test", "probe:deny --version"]
        );
        let (text, green) = render(&report);
        assert!(!green);
        assert!(text.contains("gate: deny FAILED: its tool is not installed"));
        assert!(text.contains("cargo install cargo-deny --version 0.20.2 --locked"));
    }

    #[test]
    fn a_report_that_skipped_steps_is_never_green() {
        // Defends `render` on its own: a report cut short must not read green
        // even if every line in it passed.
        let (_, green) = render(&[("fmt", StepResult::Passed)]);
        assert!(!green);
    }

    #[test]
    fn onedrive_without_a_target_dir_warns_and_names_the_fix() {
        // Forward slashes are a separator on every platform; a backslash on
        // Linux or macOS is part of one component, and the test would fail there.
        let root = Path::new("/Users/someone/OneDrive/Masaüstü/ephesus");
        let warning = target_dir_warning(root, None).unwrap();
        assert!(warning.contains("CARGO_TARGET_DIR is not set"));
        assert!(warning.contains("outside OneDrive"));
        // An empty value is the same as unset.
        assert!(target_dir_warning(root, Some(&OsString::new())).is_some());
    }

    #[test]
    fn a_set_target_dir_or_a_path_outside_onedrive_is_silent() {
        let inside = Path::new("/Users/someone/OneDrive - Work/ephesus");
        let set = OsString::from("/cargo-target");
        assert_eq!(target_dir_warning(inside, Some(&set)), None);
        assert!(target_dir_warning(inside, None).is_some());
        let outside = Path::new("/home/someone/src/ephesus");
        assert_eq!(target_dir_warning(outside, None), None);
        // A folder merely containing the word is not OneDrive.
        let lookalike = Path::new("/home/someone/notOneDrive/ephesus");
        assert_eq!(target_dir_warning(lookalike, None), None);
    }
}

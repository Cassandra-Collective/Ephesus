//! `cargo xtask mutants <package>`: a mutation round and its control, as one
//! command (GYM-008's rules, in Rust; docs/RUST-PLAN.md R0.2).
//!
//! Contract: runs cargo-mutants twice over one package, with the committed spec
//! `.cargo/mutants.toml`, and returns a verdict. It writes only under the system
//! temp directory (cargo-mutants mutates a copy of the tree there, and its
//! report goes there too), and it unsets `CARGO_TARGET_DIR` for both runs so
//! each copy builds in its own target. The rules it holds, each from a round
//! that once reported a score it had not earned (memory: *a mutation round needs
//! a control*):
//!
//! 1. **The baseline must pass.** An unmutated tree that fails its own tests
//!    makes every mutant look caught.
//! 2. **The control must report misses.** The control is the same round over one
//!    shard with a test filter that matches no test. Every viable mutant must
//!    then be MISSED. If the control catches one, the harness reports kills it
//!    did not earn, and the round is INVALID, not perfect.
//! 3. **The sources must not change.** Every member's sources and manifest are
//!    compared before and after. A round that edited the tree in place proves
//!    nothing about the tree it reports on.
//! 4. **A timeout is not a kill.** A mutant that only timed out makes the round
//!    INVALID until it is re-run or the hang is fixed.
//! 5. **No viable mutant is no evidence.** A round with nothing to test is
//!    INVALID.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::gate::workspace_root;
use crate::workspace;

/// The libtest filter the control runs with. It names no test, so no test runs.
pub const CONTROL_FILTER: &str = "__ephesus_mutants_control_matches_no_test__";

/// The shard of mutants the control runs: enough to prove the harness reports a
/// miss, without doubling the round's time.
pub const CONTROL_SHARD: &str = "1/4";

/// What one cargo-mutants run reported (`mutants.out/outcomes.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    /// Mutants generated.
    pub total: u64,
    /// Mutants no test caught.
    pub missed: u64,
    /// Mutants a test caught.
    pub caught: u64,
    /// Mutants whose tests timed out.
    pub timeout: u64,
    /// Mutants that did not build.
    pub unviable: u64,
    /// Whether the unmutated baseline built and passed.
    pub baseline_ok: bool,
}

impl Totals {
    /// Mutants that built.
    pub fn viable(&self) -> u64 {
        self.total.saturating_sub(self.unviable)
    }
}

/// Reads cargo-mutants' `outcomes.json`. This is the validator for that file:
/// the one place its untyped JSON is read.
pub fn parse_outcomes(json: &str) -> Result<Totals, String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| format!("outcomes.json is not JSON: {error}"))?;
    let count = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("outcomes.json has no numeric `{key}`"))
    };
    let baseline_ok = value
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
        .and_then(|outcomes| {
            outcomes.iter().find(|outcome| {
                outcome.get("scenario").and_then(serde_json::Value::as_str) == Some("Baseline")
            })
        })
        .and_then(|baseline| baseline.get("summary"))
        .and_then(serde_json::Value::as_str)
        == Some("Success");
    Ok(Totals {
        total: count("total_mutants")?,
        missed: count("missed")?,
        caught: count("caught")?,
        timeout: count("timeout")?,
        unviable: count("unviable")?,
        baseline_ok,
    })
}

/// A round's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every viable mutant was caught, and the control proved the harness can
    /// report a miss.
    Ok,
    /// This many mutants survived: tests are missing or a mutant is equivalent.
    Survivors(u64),
    /// The round proves nothing; the reason says why.
    Invalid(String),
}

/// Judges a round by the module's five rules.
pub fn judge(round: &Totals, control: &Totals, sources_unchanged: bool) -> Verdict {
    if !sources_unchanged {
        return Verdict::Invalid(
            "the sources changed during the round, so it proves nothing about the tree it reports on"
                .to_string(),
        );
    }
    if !round.baseline_ok {
        return Verdict::Invalid(
            "the unmutated baseline did not pass, so every mutant would look caught".to_string(),
        );
    }
    if round.viable() == 0 {
        return Verdict::Invalid("the round had no viable mutant to test".to_string());
    }
    if !control.baseline_ok {
        return Verdict::Invalid("the control's baseline did not run".to_string());
    }
    if control.viable() == 0 {
        return Verdict::Invalid(format!(
            "the control's shard ({CONTROL_SHARD}) held no viable mutant, so it cannot show the harness reports a miss"
        ));
    }
    if control.missed != control.viable() {
        return Verdict::Invalid(format!(
            "with no test running, all {} viable control mutants must be missed, but {} were; \
             the harness is reporting kills it did not earn",
            control.viable(),
            control.missed
        ));
    }
    if round.timeout > 0 {
        return Verdict::Invalid(format!(
            "{} mutant(s) timed out; a timeout is not a kill. Re-run, or fix the hang the mutant exposes",
            round.timeout
        ));
    }
    if round.missed > 0 {
        return Verdict::Survivors(round.missed);
    }
    Verdict::Ok
}

/// The cargo-mutants arguments for the round over `package`, reporting to `out`.
pub fn round_args(package: &str, out: &Path) -> Vec<OsString> {
    vec![
        "mutants".into(),
        "--package".into(),
        package.into(),
        "--output".into(),
        out.into(),
        "--no-shuffle".into(),
    ]
}

/// The control's arguments: the round's, over one shard, with no test run.
pub fn control_args(package: &str, out: &Path) -> Vec<OsString> {
    let mut args = round_args(package, out);
    args.extend(
        ["--shard", CONTROL_SHARD, "--", "--", CONTROL_FILTER]
            .into_iter()
            .map(OsString::from),
    );
    args
}

/// What a round touches outside its own logic. Abstracted so the order of
/// steps and every refusal are testable without running cargo-mutants.
pub trait Harness {
    /// Runs `cargo <args>` with `CARGO_TARGET_DIR` unset; `quiet` discards its
    /// output. Returns the exit code, or `None` if it did not finish with one.
    fn cargo(&mut self, args: &[OsString], quiet: bool) -> Option<i32>;
    /// Creates `dir` and its parents.
    fn prepare(&mut self, dir: &Path) -> Result<(), String>;
    /// Every member's manifest and sources, by path: what must not change.
    fn snapshot(&mut self, root: &Path) -> Result<BTreeMap<PathBuf, String>, String>;
    /// The totals cargo-mutants wrote under `out`.
    fn totals(&mut self, out: &Path) -> Result<Totals, String>;
}

/// The directory cargo-mutants writes its report into, under `--output`.
pub const REPORT_DIR: &str = "mutants.out";

/// The real harness: cargo, the file system, `cargo metadata`.
pub struct Real {
    /// The report directory's name: [`REPORT_DIR`] in every real round. It is
    /// a field only so a test can read a committed report under another name,
    /// because cargo-mutants never copies a directory named `mutants.out` into
    /// the tree it mutates, and a fixture by that name vanished from the copy
    /// and failed round 2's baseline (2026-10-19).
    pub report_dir: &'static str,
}

impl Default for Real {
    fn default() -> Self {
        Real {
            report_dir: REPORT_DIR,
        }
    }
}

impl Harness for Real {
    #[expect(
        clippy::disallowed_methods,
        reason = "process: RUST-PLAN §3.2 allowlists xtask; a mutation round runs cargo-mutants"
    )]
    fn cargo(&mut self, args: &[OsString], quiet: bool) -> Option<i32> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
        let mut command = Command::new(cargo);
        command.args(args).env_remove("CARGO_TARGET_DIR");
        if quiet {
            command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
        }
        command.status().ok()?.code()
    }

    fn prepare(&mut self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))
    }

    fn snapshot(&mut self, root: &Path) -> Result<BTreeMap<PathBuf, String>, String> {
        let workspace = workspace::load(root).map_err(|error| error.to_string())?;
        let mut files = BTreeMap::new();
        for member in workspace.members {
            files.insert(member.manifest_path, member.manifest);
            for source in member.sources {
                files.insert(source.path, source.text);
            }
        }
        Ok(files)
    }

    fn totals(&mut self, out: &Path) -> Result<Totals, String> {
        let path = out.join(self.report_dir).join("outcomes.json");
        let json = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        parse_outcomes(&json)
    }
}

/// The install line a missing cargo-mutants is refused with.
pub const INSTALL: &str = "cargo install cargo-mutants --version 27.1.0 --locked";

/// Runs a round and its control over `package` in the workspace at `root`,
/// reporting under `base`. Returns the exit code (0 OK, 1 survivors, 3
/// INVALID) and the report, one line per step.
pub fn round(harness: &mut dyn Harness, root: &Path, package: &str, base: &Path) -> (u8, String) {
    let mut report = String::new();
    let mut say = |line: String| {
        report.push_str("mutants: ");
        report.push_str(&line);
        report.push('\n');
    };
    if harness.cargo(&["mutants".into(), "--version".into()], true) != Some(0) {
        say(format!(
            "INVALID: cargo-mutants is not installed. Install the approved version (docs/RUST-CRATES.md):\n    {INSTALL}"
        ));
        return (3, report);
    }
    let (round_out, control_out) = (base.join("round"), base.join("control"));
    // cargo-mutants creates its output directory but not that directory's
    // parent (found by the first real run of this command, 2026-10-19).
    if let Err(error) = harness.prepare(base) {
        say(format!("INVALID: {error}"));
        return (3, report);
    }
    let before = match harness.snapshot(root) {
        Ok(files) => files,
        Err(error) => {
            say(format!(
                "INVALID: cannot read the sources before the round: {error}"
            ));
            return (3, report);
        }
    };
    say(format!(
        "round over `{package}` (spec .cargo/mutants.toml), report in {}",
        round_out.display()
    ));
    let round_exit = harness.cargo(&round_args(package, &round_out), false);
    say(format!(
        "control over shard {CONTROL_SHARD} with no test run, report in {}",
        control_out.display()
    ));
    let control_exit = harness.cargo(&control_args(package, &control_out), false);
    say(format!(
        "cargo-mutants exited {} (round) and {} (control)",
        describe_exit(round_exit),
        describe_exit(control_exit)
    ));
    if let Some(reason) = tool_failure(round_exit).or_else(|| tool_failure(control_exit)) {
        say(format!("INVALID: {reason}"));
        return (3, report);
    }
    let after = match harness.snapshot(root) {
        Ok(files) => files,
        Err(error) => {
            say(format!(
                "INVALID: cannot read the sources after the round: {error}"
            ));
            return (3, report);
        }
    };
    let (round, control) = match (harness.totals(&round_out), harness.totals(&control_out)) {
        (Ok(round), Ok(control)) => (round, control),
        (Err(error), _) | (_, Err(error)) => {
            say(format!("INVALID: {error}"));
            return (3, report);
        }
    };
    let unchanged = before == after;
    say(format!(
        "round   {} mutants: {} caught, {} missed, {} timeout, {} unviable; baseline {}",
        round.total,
        round.caught,
        round.missed,
        round.timeout,
        round.unviable,
        if round.baseline_ok { "ok" } else { "FAILED" }
    ));
    say(format!(
        "control {} mutants: {} missed of {} viable (all must be)",
        control.total,
        control.missed,
        control.viable()
    ));
    say(format!(
        "sources {} ({} files compared)",
        if unchanged { "unchanged" } else { "CHANGED" },
        before.len()
    ));
    match judge(&round, &control, unchanged) {
        Verdict::Ok => {
            say("ROUND OK".to_string());
            (0, report)
        }
        Verdict::Survivors(count) => {
            say(format!(
                "{count} SURVIVOR(S), listed in {}",
                round_out.join(REPORT_DIR).join("missed.txt").display()
            ));
            (1, report)
        }
        Verdict::Invalid(reason) => {
            say(format!("INVALID: {reason}"));
            (3, report)
        }
    }
}

/// `cargo xtask mutants <package>`. Exit 0: OK. 1: survivors. 2: usage.
/// 3: INVALID.
pub fn main(args: &[String]) -> u8 {
    let Some(package) = args.first() else {
        eprintln!("usage: cargo xtask mutants <package>");
        return 2;
    };
    let base =
        std::env::temp_dir().join(format!("ephesus-mutants-{package}-{}", std::process::id()));
    let (code, report) = round(&mut Real::default(), &workspace_root(), package, &base);
    print!("{report}");
    code
}

fn describe_exit(code: Option<i32>) -> String {
    code.map_or_else(
        || "without an exit code".to_string(),
        |code| code.to_string(),
    )
}

/// Whether cargo-mutants itself failed, rather than reporting on the code.
/// Its exit codes: 0 every mutant caught, 2 some missed (both observed
/// 2026-10-19), 3 some timed out, 4 the baseline failed. Anything else (1 is
/// its usage or runtime error) means there is no report to judge.
pub fn tool_failure(code: Option<i32>) -> Option<String> {
    match code {
        Some(0 | 2 | 3 | 4) => None,
        Some(code) => Some(format!(
            "cargo-mutants failed (exit {code}) before it could report; its message is above"
        )),
        None => Some("cargo-mutants did not start, or ended without an exit code".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(total: u64, missed: u64, caught: u64, timeout: u64, unviable: u64) -> Totals {
        Totals {
            total,
            missed,
            caught,
            timeout,
            unviable,
            baseline_ok: true,
        }
    }

    const GOOD_ROUND: Totals = Totals {
        total: 31,
        missed: 0,
        caught: 26,
        timeout: 0,
        unviable: 5,
        baseline_ok: true,
    };
    const GOOD_CONTROL: Totals = Totals {
        total: 8,
        missed: 6,
        caught: 0,
        timeout: 0,
        unviable: 2,
        baseline_ok: true,
    };

    #[test]
    fn a_clean_round_with_an_honest_control_is_ok() {
        assert_eq!(judge(&GOOD_ROUND, &GOOD_CONTROL, true), Verdict::Ok);
    }

    #[test]
    fn survivors_are_counted() {
        let round = totals(31, 3, 23, 0, 5);
        assert_eq!(judge(&round, &GOOD_CONTROL, true), Verdict::Survivors(3));
    }

    #[test]
    fn each_rule_makes_the_round_invalid() {
        let mut no_baseline = GOOD_ROUND;
        no_baseline.baseline_ok = false;
        let mut control_no_baseline = GOOD_CONTROL;
        control_no_baseline.baseline_ok = false;
        let cases = [
            (GOOD_ROUND, GOOD_CONTROL, false, "sources changed"),
            (no_baseline, GOOD_CONTROL, true, "baseline did not pass"),
            (
                totals(4, 0, 0, 0, 4),
                GOOD_CONTROL,
                true,
                "no viable mutant",
            ),
            (GOOD_ROUND, control_no_baseline, true, "control's baseline"),
            (
                GOOD_ROUND,
                totals(2, 0, 0, 0, 2),
                true,
                "held no viable mutant",
            ),
            (
                GOOD_ROUND,
                totals(8, 5, 1, 0, 2),
                true,
                "kills it did not earn",
            ),
            (totals(31, 0, 25, 1, 5), GOOD_CONTROL, true, "timed out"),
        ];
        for (round, control, unchanged, needle) in cases {
            match judge(&round, &control, unchanged) {
                Verdict::Invalid(reason) => assert!(reason.contains(needle), "{needle}: {reason}"),
                other => panic!("{needle}: expected INVALID, got {other:?}"),
            }
        }
    }

    #[test]
    fn invalid_outranks_survivors() {
        // A round that also has survivors is still INVALID when the control lies.
        let round = totals(31, 3, 23, 0, 5);
        let lying_control = totals(8, 0, 6, 0, 2);
        assert!(matches!(
            judge(&round, &lying_control, true),
            Verdict::Invalid(_)
        ));
    }

    #[test]
    fn outcomes_json_is_read_with_its_baseline() {
        // The shape cargo-mutants 27.1.0 writes (read from a real report, 2026-10-19).
        let json = r#"{"outcomes":[{"scenario":"Baseline","summary":"Success"},{"scenario":{"Mutant":{}},"summary":"MissedMutant"}],
            "total_mutants":4,"missed":2,"caught":0,"timeout":0,"unviable":2,"success":0}"#;
        let parsed = parse_outcomes(json).unwrap();
        assert_eq!(
            parsed,
            Totals {
                total: 4,
                missed: 2,
                caught: 0,
                timeout: 0,
                unviable: 2,
                baseline_ok: true
            }
        );
        assert_eq!(parsed.viable(), 2);
        let failed = json.replace("\"Success\"", "\"Failure\"");
        assert!(!parse_outcomes(&failed).unwrap().baseline_ok);
        let no_baseline =
            r#"{"outcomes":[],"total_mutants":0,"missed":0,"caught":0,"timeout":0,"unviable":0}"#;
        assert!(!parse_outcomes(no_baseline).unwrap().baseline_ok);
    }

    #[test]
    fn a_report_missing_a_count_or_not_json_is_an_error() {
        assert!(parse_outcomes("not json").unwrap_err().contains("not JSON"));
        let missing = r#"{"outcomes":[],"total_mutants":1,"caught":0,"timeout":0,"unviable":0}"#;
        assert!(parse_outcomes(missing).unwrap_err().contains("`missed`"));
    }

    #[test]
    fn a_reporting_exit_is_judged_and_a_tool_failure_is_not() {
        for code in [0, 2, 3, 4] {
            assert_eq!(tool_failure(Some(code)), None, "{code}");
        }
        assert!(tool_failure(Some(1)).unwrap().contains("exit 1"));
        assert!(tool_failure(None).unwrap().contains("did not start"));
    }

    #[test]
    fn the_control_is_the_round_over_one_shard_with_no_test() {
        let out = Path::new("/tmp/out");
        let round = round_args("xtask", out);
        let control = control_args("xtask", out);
        assert_eq!(&control[..round.len()], &round[..]);
        let tail: Vec<&str> = control[round.len()..]
            .iter()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(tail, ["--shard", CONTROL_SHARD, "--", "--", CONTROL_FILTER]);
        let round: Vec<&str> = round.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            round,
            [
                "mutants",
                "--package",
                "xtask",
                "--output",
                "/tmp/out",
                "--no-shuffle"
            ]
        );
    }

    // ---- the round's orchestration, through a scripted harness ----------

    /// Plays a harness from a script and records what it was asked.
    struct Scripted {
        installed: bool,
        round_exit: Option<i32>,
        control_exit: Option<i32>,
        prepare: Result<(), String>,
        /// The snapshots returned, in order (before, after).
        snapshots: Vec<Result<BTreeMap<PathBuf, String>, String>>,
        totals: Vec<Result<Totals, String>>,
        calls: Vec<String>,
    }

    fn files(text: &str) -> BTreeMap<PathBuf, String> {
        BTreeMap::from([(PathBuf::from("src/lib.rs"), text.to_string())])
    }

    impl Scripted {
        fn green() -> Self {
            Scripted {
                installed: true,
                round_exit: Some(0),
                control_exit: Some(2),
                prepare: Ok(()),
                snapshots: vec![Ok(files("a")), Ok(files("a"))],
                totals: vec![Ok(GOOD_ROUND), Ok(GOOD_CONTROL)],
                calls: Vec::new(),
            }
        }
    }

    impl Harness for Scripted {
        fn cargo(&mut self, args: &[OsString], quiet: bool) -> Option<i32> {
            let line: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.calls.push(format!(
                "cargo{}:{}",
                if quiet { "(quiet)" } else { "" },
                line.join(" ")
            ));
            if line == ["mutants", "--version"] {
                return if self.installed { Some(0) } else { Some(101) };
            }
            if line.iter().any(|a| a == "--shard") {
                self.control_exit
            } else {
                self.round_exit
            }
        }

        fn prepare(&mut self, dir: &Path) -> Result<(), String> {
            self.calls.push(format!("prepare:{}", dir.display()));
            self.prepare.clone()
        }

        fn snapshot(&mut self, _root: &Path) -> Result<BTreeMap<PathBuf, String>, String> {
            self.calls.push("snapshot".to_string());
            self.snapshots.remove(0)
        }

        fn totals(&mut self, out: &Path) -> Result<Totals, String> {
            self.calls.push(format!(
                "totals:{}",
                out.file_name().unwrap().to_string_lossy()
            ));
            self.totals.remove(0)
        }
    }

    fn run_scripted(harness: &mut Scripted) -> (u8, String) {
        round(harness, Path::new("/ws"), "xtask", Path::new("/tmp/base"))
    }

    #[test]
    fn a_green_round_runs_every_step_in_order_and_reports_ok() {
        let mut harness = Scripted::green();
        let (code, report) = run_scripted(&mut harness);
        assert_eq!(code, 0, "{report}");
        assert!(report.ends_with("mutants: ROUND OK\n"), "{report}");
        assert!(
            report.contains("exited 0 (round) and 2 (control)"),
            "{report}"
        );
        assert!(
            report.contains("sources unchanged (1 files compared)"),
            "{report}"
        );
        let base = Path::new("/tmp/base");
        assert_eq!(
            harness.calls,
            [
                "cargo(quiet):mutants --version".to_string(),
                format!("prepare:{}", base.display()),
                "snapshot".to_string(),
                format!(
                    "cargo:mutants --package xtask --output {} --no-shuffle",
                    base.join("round").display()
                ),
                format!(
                    "cargo:mutants --package xtask --output {} --no-shuffle --shard 1/4 -- -- {CONTROL_FILTER}",
                    base.join("control").display()
                ),
                "snapshot".to_string(),
                "totals:round".to_string(),
                "totals:control".to_string(),
            ]
        );
    }

    #[test]
    fn survivors_exit_1_and_name_the_list() {
        let mut harness = Scripted::green();
        harness.round_exit = Some(2);
        harness.totals = vec![Ok(totals(31, 3, 23, 0, 5)), Ok(GOOD_CONTROL)];
        let (code, report) = run_scripted(&mut harness);
        assert_eq!(code, 1, "{report}");
        assert!(report.contains("3 SURVIVOR(S), listed in"), "{report}");
        assert!(report.contains("missed.txt"), "{report}");
    }

    /// A change to a green harness that one refusal must catch.
    type Plant = Box<dyn Fn(&mut Scripted)>;

    #[test]
    fn each_refusal_exits_3_and_says_why() {
        let cases: Vec<(&str, Plant)> = vec![
            (
                "cargo-mutants is not installed",
                Box::new(|h| h.installed = false),
            ),
            (
                "cannot create",
                Box::new(|h| h.prepare = Err("cannot create /tmp/base: denied".to_string())),
            ),
            (
                "before the round",
                Box::new(|h| h.snapshots[0] = Err("unreadable".to_string())),
            ),
            (
                "after the round",
                Box::new(|h| h.snapshots[1] = Err("unreadable".to_string())),
            ),
            (
                "cargo-mutants failed (exit 1)",
                Box::new(|h| h.round_exit = Some(1)),
            ),
            ("did not start", Box::new(|h| h.control_exit = None)),
            (
                "cannot read outcomes",
                Box::new(|h| h.totals[1] = Err("cannot read outcomes".to_string())),
            ),
            (
                "sources changed during the round",
                Box::new(|h| h.snapshots[1] = Ok(files("b"))),
            ),
        ];
        for (needle, plant) in cases {
            let mut harness = Scripted::green();
            plant(&mut harness);
            let (code, report) = run_scripted(&mut harness);
            assert_eq!(code, 3, "{needle}: {report}");
            assert!(report.contains("INVALID"), "{needle}: {report}");
            assert!(report.contains(needle), "{needle}: {report}");
        }
    }

    #[test]
    fn a_missing_tool_names_its_install_line_and_runs_nothing_else() {
        let mut harness = Scripted::green();
        harness.installed = false;
        let (_, report) = run_scripted(&mut harness);
        assert!(report.contains(INSTALL), "{report}");
        assert_eq!(harness.calls, ["cargo(quiet):mutants --version"]);
    }

    #[test]
    fn changed_sources_are_reported_as_changed() {
        let mut harness = Scripted::green();
        harness.snapshots[1] = Ok(files("b"));
        let (_, report) = run_scripted(&mut harness);
        assert!(report.contains("sources CHANGED"), "{report}");
    }

    #[test]
    fn an_exit_without_a_code_is_described_not_hidden() {
        assert_eq!(describe_exit(Some(2)), "2");
        assert_eq!(describe_exit(None), "without an exit code");
    }

    // ---- the real harness's halves ----------------------------------------

    #[test]
    fn the_real_harness_reads_a_report_cargo_mutants_wrote() {
        // The committed report lives under `report/`, because cargo-mutants
        // does not copy a `mutants.out` directory into the tree it mutates.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/outcomes");
        let mut harness = Real {
            report_dir: "report",
        };
        let totals = harness.totals(&fixture).unwrap();
        assert_eq!(
            totals,
            Totals {
                total: 3,
                missed: 1,
                caught: 1,
                timeout: 0,
                unviable: 1,
                baseline_ok: true
            }
        );
        let missing = harness.totals(Path::new("no-such-report")).unwrap_err();
        assert!(missing.contains("cannot read"), "{missing}");
    }

    #[test]
    fn a_real_round_reads_the_directory_cargo_mutants_writes() {
        assert_eq!(Real::default().report_dir, "mutants.out");
        assert_eq!(REPORT_DIR, "mutants.out");
    }

    #[test]
    fn the_real_harness_snapshots_this_workspace() {
        let files = Real::default().snapshot(&workspace_root()).unwrap();
        let this_file = files
            .keys()
            .find(|path| path.ends_with(Path::new("src").join("mutants.rs")))
            .unwrap();
        assert!(files[this_file].contains("pub trait Harness"));
        assert!(files.keys().any(|path| path.ends_with("Cargo.toml")));
    }

    #[test]
    fn the_real_harness_runs_cargo_and_returns_its_code() {
        assert_eq!(Real::default().cargo(&["--version".into()], true), Some(0));
        let unknown = Real::default().cargo(&["ephesus-no-such-subcommand".into()], true);
        assert!(
            matches!(unknown, Some(code) if code != 0 && code != 1),
            "{unknown:?}"
        );
    }

    #[test]
    fn the_real_harness_prepares_a_directory_and_its_parents() {
        let dir = std::env::temp_dir()
            .join(format!("ephesus-xtask-prepare-{}", std::process::id()))
            .join("a")
            .join("b");
        Real::default().prepare(&dir).unwrap();
        assert!(dir.is_dir());
        std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).unwrap();
    }
}

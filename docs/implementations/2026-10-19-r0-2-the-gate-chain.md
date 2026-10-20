# R0.2 — The invariant checks, the dependency check, the mutation command and the Rust CI job

**Date:** 2026-10-19 · **Branch:** `feature/rust-rewrite` · **Package:** R0.2
(`docs/RUST-PLAN.md` §4, §5) · **Decisions:** DECISIONS-LOG 2026-10-19, the two
R0.2 entries (CI on three platforms; `toml` and `proc-macro2` by memo; first-party CI
tooling; the token and prompt-literal rules arrive with their subjects; a Rust
toolchain in WSL) · **Gymnasium:** GYM-013 metric 4; GYM-008's metric 1 kept true
for Rust

## 1. Problem / motivation

R0.1 gave the workspace lints that clippy enforces. Clippy cannot see four things
ADR-0041's invariant table needs:

- **Whether a crate opted in at all.** The workspace lints reach a crate only if its
  manifest says `[lints] workspace = true`. A crate that forgets is unlinted in silence.
- **The crate graph.** RUST-PLAN §3.2's trust boundary, where the window reaches the
  core through `eph-api` alone, is a property of the dependency graph, not of any one
  call.
- **What an exemption excuses.** One clippy lint, `disallowed_methods`, covers both
  process starts and direct file writes. So `eph-git`, allowed to start git, could
  write a file through the same exemption.
- **Which process is started.** Clippy refuses `Command::new` outside an exemption, but
  inside one it cannot tell `cargo` from `git`. Invariant 4, the single committer, is
  about which program.

Invariant 10 had no machine check either. GYM-008's landed metric ("every round
cites a checked-in spec") had no Rust form, and CI did not run the Rust gate at all.

## 2. What changed

| File | Change |
|---|---|
| `xtask/src/workspace.rs` | New. The workspace as plain data: members, manifests, direct dependencies, sources. Read once via `cargo metadata --no-deps --locked`; the checks never touch the disk. |
| `xtask/src/invariants.rs` | New. Six rules: `lints`, `dep-shape`, `no-io`, `exemption`, `git`, `parse`. Each violation names its file, line and rule. RUST-PLAN §3.2 is a `POLICIES` table. |
| `xtask/src/deps.rs` | New. Invariant 10: every direct dependency, in every table, is a member or an approved crate. A path dependency outside the workspace is refused. `APPROVED` is pinned to RUST-CRATES.md by a test. |
| `xtask/src/mutants.rs` | New. `cargo xtask mutants <package>`: the round, its control, a before/after source comparison, and a verdict by five rules. |
| `xtask/src/gate.rs` | The chain gains `invariants` and `deps` (in-process steps), so a failing check points at its findings. The OneDrive tests use forward-slash paths, which are separators on every platform. |
| `xtask/src/lib.rs` | Dispatches `invariants`, `deps` and `mutants`. `run_check` refuses a check that read no crate or no file. |
| `xtask/tests/cli.rs` | Three more integration tests: both checks pass over this workspace and say what they read, `mutants` with no package is a usage error, and the real runner returns each check's verdict on the fixtures. |
| `xtask/tests/fixtures/` | New, committed: `clean/` and `violating/` workspaces, a package `outside/` the violating one, and a real-shaped cargo-mutants report. No test writes a file. A fixture holds its own `Cargo.toml`, so the loader skips it as another package. |
| `clippy.toml` | `std::fs::OpenOptions::open` and `std::fs::copy` added to the disallowed list. Both write a live file in place, and invariant 3 had no rule against either (found while planning round 2). |
| `xtask/src/main.rs` | `ExitCode::from(dispatch(..))`. Every command now returns a plain `u8`, so the tests assert each exit code in-process. |
| `xtask/Cargo.toml`, `Cargo.toml`, `Cargo.lock` | `[workspace.dependencies]`: `cargo_metadata`, `proc-macro2` (`span-locations`), `serde_json`, `syn` (`full`, `visit`), `toml`. All approved. |
| `.cargo/mutants.toml` | New. The committed spec cargo-mutants reads, with no exclusions. |
| `.github/workflows/ci.yml` | New `rust` job on `ubuntu-latest`, `windows-latest` and `macos-latest`. Toolchain from `rust-toolchain.toml`, cargo-deny 0.20.2 via `cargo install --locked`, `actions/cache` only, and the gate through the `--locked` alias. |
| `.prettierignore` | `xtask/` and `crates/`: rustfmt formats the Rust workspace. Without this, the TypeScript CI's required `prettier --check .` failed on the one-line `outcomes.json` fixture (reproduced before the fix, clean after; §6). |
| `Cargo.toml` (`[profile.mutants]`) | The test profile without debug info, which a mutation round builds with. |
| `docs/RUST-CRATES.md` | Memo rows for `toml` and `proc-macro2`. |
| `docs/RUST-PLAN.md`, `docs/DECISIONS-LOG.md` | The R0.2 decisions. |
| `docs/PROGRESS.md` | R0.2's box. |

## 3. Implementation approach

**Policy as data.** RUST-PLAN §3.2 is the `POLICIES` table. Each entry gives a crate's
allowed workspace dependencies, allowed external dependencies, whether it may do I/O,
which exemption categories it may claim, and the paths it must not name. Entries name
crates that do not exist yet, so a rule is in force from the commit that creates its
crate. An unlisted crate gets `DEFAULT_POLICY`: anything approved, I/O allowed, no
exemption.

**The exemption rule is category-aware.** An exemption's `reason` must begin with
`process:`, `git:` or `fs-write:`. The crate's policy must allow that category. The
calls under the exemption, found by name, must match it: a `process:` exemption that
also covers a `fs::write` is refused. An exemption must sit on a fn, impl, module or
file, where the rule can see what it covers. One on a statement or an expression is
refused. `#[allow]` is refused, because an `#[expect]` fails the build once its call is
gone. `cfg_attr` is opened, so `#[cfg_attr(test, allow(clippy::disallowed_methods))]`
does not hide.

**The single-committer rule, refuted with GYM-009's catalogue.** The TypeScript
tripwire's history (GYM-009, GYM-010) produced a list of the shapes a git start
takes. `invariants.rs` carries that list as tests in two halves. Shapes it sees (16):
a literal, an imported `Command`, `tokio`'s `Command`, a reference, a same-file
`const` and `static`, the binary by POSIX and Windows path, `git.exe`, `GIT`, an
import alias, a grouped alias, `sh -c`, `cmd /C` via `args`, `pwsh` via `vec!`, and a
Prettier-style wrapped builder. Shapes it does not see are pinned as limits: a
variable, `format!`, a function result, an environment value. Each limit is a test
that asserts the rule stays blind, so a fix moves it to the first half on purpose.

**Line numbers.** `syn` spans carry positions only with `proc-macro2`'s
`span-locations`. The Architect added `proc-macro2` by memo, because GYM-013 metric 4
promises "by file and line".

**The mutation command holds GYM-008's rules.** The round runs with
`.cargo/mutants.toml`. The control is the same round over shard 1/4, with a libtest
filter that matches no test, so every viable mutant must be missed. Before writing
this, a probe in a scratch copy confirmed that cargo-mutants accepts a baseline that
runs zero tests, and that `-- -- <filter>` reaches the test binary. Its command line
read `test --verbose --package=xtask@0.0.0 -- __ephesus_mutants_control_matches_no_test__`.
The verdict is INVALID when:

- the sources changed;
- the baseline failed;
- nothing was viable;
- the control caught anything;
- anything timed out;
- or cargo-mutants itself failed.

## 4. Mathematical / statistical details

Not applicable: a structural change. The control's sample is one shard of four, a
deliberate trade: it proves the harness reports a miss without doubling the round's
time.

## 5. Design decisions

- **`run_check` refuses a check over nothing.** The memory rule that a run with zero
  tests is INVALID, not a pass, applied to checks: zero crates or zero files is a
  failure, and the success line says how much was read.
- **The machine copy of the memo is pinned, not parsed at runtime.** `APPROVED` is a
  constant, and a test compares it with RUST-CRATES.md's Set A tables. A memo edit
  without a code edit, or the reverse, fails the suite. Parsing markdown at gate time
  would make a formatting change a gate failure.
- **Dev dependencies are free of `dep-shape` but not of `deps`.** A test helper may not
  leak into the shipped crate's graph, but it is still a dependency that needs a memo.
- **`cargo metadata --no-deps --locked`.** `--no-deps` never resolves the registry, so
  the checks run offline. `--locked` keeps the hole R0.1 closed shut here too.

## 6. Verification

Win32 (Windows 11 Home 10.0.26300), toolchain 1.96.0-x86_64-pc-windows-msvc. Tier A
unless marked.

**The gate:** all six steps green. 76 unit and 8 integration tests at the final tree
(48 and 7 when first green, before round 1's survivors were answered):

```
invariants: ok (1 crate(s), 8 source file(s))
deps: ok (1 crate(s), 8 source file(s))
gate: fmt ok
gate: clippy ok
gate: test ok
gate: deny ok
gate: invariants ok
gate: deps ok
gate: all steps green
```

**A test that caught its own fixture.** The first lints fixture wrote
`lints.workspace = true` under `[package]`, where it means `package.lints.workspace`,
a key cargo ignores. The rule refused it, correctly. The fixture was fixed, and that
shape is now a must-fail case. A text match on the line would have passed it.

**The rules on a real tree, through real `cargo metadata`.** In a scratch copy under
`%TEMP%`, never committed, plants were made in two new crates, `eph-ui` and
`eph-schema`:

```
crates\eph-schema\Cargo.toml:1: [lints] `eph-schema` does not opt into the workspace lints …
crates\eph-schema\src\lib.rs:4: [no-io] `eph-schema` names `std::process::Command::new`, which is I/O …
crates\eph-schema\src\lib.rs:2: [exemption] `eph-schema` may not claim `fs-write:` (… allows it: none)
crates\eph-schema\src\lib.rs:4: [git] `eph-schema` starts git, and only the single committer may …
crates\eph-ui\Cargo.toml:10: [dep-shape] `eph-ui` may depend only on eph-api of this workspace …, but [dependencies] names `eph-fs`
crates\eph-ui\Cargo.toml:11: [dep-shape] … names `left-pad`
crates\eph-ui\src\lib.rs:2: [no-io] `eph-ui` names `std::fs` …
crates\eph-ui\src\lib.rs:3: [no-io] `eph-ui` names `alacritty_terminal::tty::Pty`, which is banned in this crate
crates\eph-ui\src\lib.rs:5: [exemption] use `#[expect(...)]`, not `#[allow]` …
invariants: 10 violation(s)
```

The `eph-schema` lints plant was the `[package]`-scoped dotted key, refused on the
real tree too. `deps` passed that run, and correctly: cargo makes a path dependency
inside the workspace directory a member, so `vendor/left-pad` was ours. Re-planted
offline:

```
== plant A: a path dependency outside the workspace
xtask\Cargo.toml:17: [deps] `xtask` depends on `left-pad` by path, and it is not a member …
== plant B: an unapproved registry crate, lockfile not updated
xtask\Cargo.toml:17: [deps] `xtask` depends on `left-pad` in [dependencies], and no approved set … lists it
rc=1 lock before=c6a0d51acde4 after=c6a0d51acde4
== plant C: a dev-dependency
xtask\Cargo.toml:22: [deps] … in [dev-dependencies] …
```

**Mutation round 1** (`cargo xtask mutants xtask`, win32), verbatim from the command:

```
mutants: cargo-mutants exited 2 (round) and 2 (control)
mutants: round   218 mutants: 170 caught, 34 missed, 0 timeout, 14 unviable; baseline ok
mutants: control 55 mutants: 53 missed of 53 viable (all must be)
mutants: sources unchanged (9 files compared)
mutants: 34 SURVIVOR(S), listed in …oundmutants.outmissed.txt
```

The control was honest: with no test running, all 53 viable mutants were missed. The
exit code 2 for missed mutants, which `tool_failure` assumed, was observed. The 34
survivors were each a missing test, or in two cases a design smell:

- **Orchestration with no test (13).** `mutants::main`, the real cargo call, the
  snapshot, the report reader, `describe_exit` and `invalid` ran only in the real
  command. They now sit behind a `Harness` trait. A scripted harness tests the order
  of steps and every refusal (eight of them, each exiting 3 and saying why). The
  real harness's halves are tested on their own: reading the committed report,
  snapshotting this workspace, running `cargo --version` and an unknown subcommand,
  and creating a nested directory.
- **The real runner's `check`, and the failure exit path (6).** Now tested against
  the committed fixtures, along with each command's exit code through `dispatch`. The
  `||` → `&&` mutant in the "check over nothing" guard is killed by a pure
  `read_nothing` with all four cases.
- **The loader (5).** Dependency kinds, member detection, the `target/` and dot-
  directory skip, and `LoadError`'s messages had no test against a real `cargo
  metadata`. The fixtures give them one.
- **The exemption rule (6).** No fixture placed an exemption on a `mod` or a trait
  fn, used `disallowed_types`, or used a name-value other than `reason`. None
  exercised `.truncate` through the method visitor, or a two-segment `fs::write`.
  The last is why `- → /` in `ends_with` survived: for 3- and 4-segment paths,
  `n - 2 == n / 2`.
- **The git rule (4).** `Command::describe("git")` and a parenthesised builder had no
  case. Two survivors were design smells, so their code changed rather than gaining a
  test. `path == "Command"` in alias detection could tell nothing real apart, and is
  deleted. The `vec!`-only guard on macro arguments made the rule blind to
  `format!("git {}", x)` handed to a shell, so any macro's comma body is read now, and
  that shape moved from the limits to the cases the rule sees.

**Mutation round 2 tested nothing, and said so.** Its baseline failed inside
cargo-mutants' copy of the tree:

```
test mutants::tests::the_real_harness_reads_a_report_cargo_mutants_wrote ... FAILED
cannot read …\cargo-mutants-…tmp\xtask\tests/fixtures/outcomes\mutants.out\outcomes.json: The system cannot find the path specified.
ERROR cargo test failed in an unmutated tree, so no mutants were tested
```

cargo-mutants never copies a directory named `mutants.out` into the tree it mutates,
and the committed report fixture lived in one. It does not copy `target/` either, so
the fixture's `target/junk.rs` is absent in the copy too. Round 2 then died when the
machine slept, before `judge` could print a verdict. Its first rule ("the baseline must
pass") is what it would have printed. Both are now testable wherever the tests run:

- `Real` carries the report directory's name, defaulting to
  `REPORT_DIR = "mutants.out"`, pinned by a test. The fixture moved to
  `outcomes/report/`.
- The directory-skip rule became a pure predicate, `skips_dir`, with seven cases. The
  first edit to wire it into the walk did not apply, and the gate stayed green with
  the predicate tested and unused. Reading the walk before the round caught it, and it
  is now wired (`workspace.rs`, `rust_files`). That is memory's *a green suite is not
  a wired feature*, at the size of one function.

**Speed, with its condition.** The first probe after the fix (one shard of 100, win32)
ran the baseline in a 172 s build and a 63 s test, then each mutant in a 60–115 s build
and a 15–20 s test. That put a full round at hours. Free RAM was 1.1 GB, with other
desktop apps and sessions holding the rest. A `[profile.mutants]` (the test profile
with `debug = "none"`, named in `.cargo/mutants.toml`) brought the same probe to a 35 s
baseline build and per-mutant builds of 3 s with tests of 1–6 s. Free RAM was 1.9 GB on
that second run, so not all of the difference is the profile.

**Mutation round 3, at the final tree: ROUND OK** (`cargo xtask mutants xtask`, win32,
`[profile.mutants]`, the machine held awake), verbatim:

```
mutants: cargo-mutants exited 0 (round) and 2 (control)
mutants: round   225 mutants: 211 caught, 0 missed, 0 timeout, 14 unviable; baseline ok
mutants: control 57 mutants: 55 missed of 55 viable (all must be)
mutants: sources unchanged (9 files compared)
mutants: ROUND OK
```

All five rules held: the baseline passed in the copy, the control missed every viable
mutant, the sources were unchanged, nothing timed out, and the round had viable
mutants. Exit code 0 for a clean round and 2 for missed mutants were both observed,
which `tool_failure` assumes.

**The first real run of `cargo xtask mutants` found a defect no unit test could.**
cargo-mutants creates its output directory but not that directory's parent. The
command refused, correctly (`INVALID: cannot read …outcomes.json`), because there was
no report to read. It now creates the parent first, and it reports cargo-mutants' own
exit codes and treats a tool failure as INVALID.

**Linux (tier A, WSL2 Ubuntu, kernel 6.6.114.1, cargo 1.96.0).** The final tree was
copied into the WSL home and run, outside OneDrive and outside Windows:

```
fmt ok
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 8.66s     (clippy -D warnings)
test result: ok. 76 passed; 0 failed; …
test result: ok. 8 passed; 0 failed; …
invariants: ok (1 crate(s), 8 source file(s))
deps: ok (1 crate(s), 8 source file(s))
```

That includes the OneDrive tests, whose backslash fixture paths would have failed on
POSIX before the forward-slash fix. cargo-deny was not run on Linux, because it is not
installed in WSL. Its verdict concerns the dependency graph, which `all-features` makes
the same on every platform, and it was green on win32.

**Cross-gate hazard, caught before the commit.** `main`'s required TypeScript check runs
`prettier --check .`, and `.prettierignore` did not cover `xtask/`. Run against the
staged tree:

```
[warn] xtask/tests/fixtures/outcomes/report/outcomes.json
[warn] Code style issues found in the above file. Run Prettier with --write to fix.
```

With `xtask/` and `crates/` ignored it reads `All matched files use Prettier code style!`,
and `eslint . --max-warnings 0` passes. A Rust change can turn a TypeScript gate red,
so every R package runs the frozen tree's lint over its own diff.

**CI, all three platforms (tier B).** The push of `970ac0b` to `feature/r0-2-gate-chain`
ran workflow run 39312750826, which completed with success:

```
Rust gate (ubuntu-latest):  success (2 min)
Rust gate (windows-latest): success (3 min)
Rust gate (macos-latest):   success (3 min)
Typecheck · lint · test:    success (2 min)
Docs integrity:             success
Commit attribution:         success
```

That is the whole chain, cargo-deny included, through the `--locked` alias, on each OS
NFR-11 names. The cold-cache `cargo install cargo-deny` fits inside those minutes. The
required TypeScript check is green on this tree, so the `.prettierignore` fix holds in
CI too.

**Not proven here:** the same run with a warm cache, which only makes it faster, and any
macOS run outside CI.

# R0.1 — The Rust workspace skeleton, and lints that are proved to fire

**Date:** 2026-10-19 · **Branch:** `feature/rust-rewrite` · **Package:** R0.1
(`docs/RUST-PLAN.md` §5) · **Decisions:** ADR-0041; DECISIONS-LOG 2026-10-19
(plan approved) · **Gymnasium:** GYM-013 metric 4 (every rule refuted)

## 1. Problem / motivation

The rewrite has nowhere to live until a Cargo workspace exists, and the workspace is
where invariants 1, 3, 4 and 7 first become machine-checked in Rust (ADR-0041's table).
A lint configuration fails *silently* when it is wrong: a misspelt clippy key, a
`disallowed-methods` path that resolves to nothing, or a crate that never opts into the
workspace lints all leave the gate green while it checks nothing. Memory records this
shape three times (*Refute a new gate before closing*, *A per-line tripwire is blind to
wrapping*, *A mutation round needs a control*). So R0.1 is not done when the gate is
green. It is done when every rule has been seen to refuse a planted violation.

## 2. What changed

| File | Change |
|---|---|
| `Cargo.toml` | New. The workspace (resolver 3, edition 2024, MSRV 1.96) and `[workspace.lints]`, the Rust form of invariants 1 and 7, plus `disallowed_*` at deny. |
| `Cargo.lock` | New. Committed, because the gate builds `--locked` and a lockfile change is reviewed (ENGINEERING-STANDARDS). |
| `rust-toolchain.toml` | New. Pins 1.96.0 with rustfmt and clippy, so a lint or format rule never changes under the gate without a commit. |
| `clippy.toml` | New. Tests may unwrap, expect and panic. `std::fs::write`, `File::create`, `OpenOptions::truncate` and `std::process::Command::new` are refused, each with a reason naming its invariant and the allowed path. |
| `deny.toml` | New. cargo-deny's license allowlist (RUST-CRATES.md, approved), advisories, and sources limited to crates.io. |
| `.cargo/config.toml` | New. The `cargo xtask` alias. It deliberately does not set the target directory, which is per machine. |
| `.gitignore` | `/target/` for a build without `CARGO_TARGET_DIR`. |
| `xtask/Cargo.toml`, `xtask/src/lib.rs`, `xtask/src/main.rs`, `xtask/src/gate.rs` | New. `cargo xtask gate` runs fmt → clippy → test → deny and stops at the first red step. A missing tool fails with its install command. A build inside OneDrive with no `CARGO_TARGET_DIR` warns, naming the fix. The binary is a thin shell over `xtask::dispatch`. 9 unit tests. |
| `xtask/tests/cli.rs` | New. 5 integration tests that drive the real binary and the real `CargoRunner`, with xtask itself standing in for cargo. They are the only tests that can catch a gate reporting green without running anything (§6). |
| `docs/PROGRESS.md` | R0.1 ticked with its evidence. |
| `docs/implementations/2026-10-19-r0-1-workspace-skeleton.md` | This document. |

## 3. Implementation approach

**The chain is data, the runner is a seam.** `STEPS` is a constant table, and `run`
walks it through a `Runner` trait, which the tests replace with a script. The tests can
then pin the order, the stop-at-first-red rule and the missing-tool message without
starting cargo. The real runner is a few lines and is exercised by every
`cargo xtask gate`.

**`--locked` on every build step, and on the alias that starts the gate.** A stale
`Cargo.lock` fails the gate instead of being rewritten in silence. That is invariant
10's sibling: the dependency graph changes only in a reviewed commit. The first version
put `--locked` on the chain's steps only, and the claim was false. The `cargo xtask`
alias is itself a `cargo run`, which rewrote the lockfile before the first step ran.
A review caught it, and §6 measures both versions.

**A refusal teaches.** Each `clippy.toml` reason names the invariant and the allowed
path, the missing-tool line prints the approved install command, and the OneDrive
warning names the variable and an example value.

**The real runner is tested from outside.** The unit tests replace the runner, so they
cannot see the real one, or either entry point (memory: *a seam test does not test the
resolver*). `xtask/tests/cli.rs` points `CARGO` at the xtask binary itself. The gate's
first step then runs `xtask fmt --all --check`, an unknown command that exits 2, and
the real chain must report it red and exit 1. The mutation round's control (§6) shows
what this alone catches.

**The one allowed process start in xtask** is `CargoRunner::command`. It carries
`#[expect(clippy::disallowed_methods, reason = …)]`, not `#[allow]`, so the exemption
itself fails the build if the call ever goes away. An exemption cannot outlive what
it excuses.

## 4. Mathematical / statistical details

Not applicable: a structural change.

## 5. Design decisions

- **xtask uses only `std` in R0.1.** `syn` and `cargo_metadata` are approved, but they
  come with the rules that need them (R0.2), so R0.1's lockfile holds no dependency
  nothing uses.
- **`missing_docs` at deny, not warn.** The global standard asks every function to state
  its contract. A warning under `-D warnings` is an error anyway, and stating `deny`
  says so in the file.
- **OneDrive is a warning, not a refusal.** A slow build is a degradation, not a broken
  invariant. It is printed every run (invariant 7), not swallowed.
- **No `[workspace.dependencies]` yet.** It arrives with the first shared dependency
  (R0.2), so the table never lists a crate no member uses.

## 6. Verification

Win32 (Windows 11 Home 10.0.26300), toolchain 1.96.0-x86_64-pc-windows-msvc,
`CARGO_TARGET_DIR=%LOCALAPPDATA%\ephesus-cargo-target`. All tier A, this session.

**The refutation pass: one planted violation per rule.** A temporary function in
`xtask/src/main.rs`, removed after the run, held one instance of each rule.
`cargo clippy --workspace --all-targets --locked -- -D warnings` refused every one,
each by its own rule:

```
error: use of a disallowed method `std::fs::write`
error: use of a disallowed method `std::fs::File::create`
error: use of a disallowed method `std::fs::OpenOptions::truncate`
error: use of a disallowed method `std::process::Command::new`
error: used `unwrap()` on an `Option` value
error: used `expect()` on an `Option` value
error: non-binding `let` on an expression with `#[must_use]` type
error: `panic` should not be present in production code
error: `todo` should not be present in production code
error: `unimplemented` should not be present in production code
error: the `dbg!` macro is intended as a debugging tool
error: usage of an `unsafe` block
error: missing documentation for a function
```

13 of 13 refused. With the plant removed, clippy is green again.

**The gate, with cargo-deny not yet installed.** It refused, teaching the fix:

```
gate: fmt ok
gate: clippy ok
gate: test ok
gate: deny FAILED: its tool is not installed. Install the approved version (docs/RUST-CRATES.md):
    cargo install cargo-deny --version 0.20.2 --locked
gate: RED
```

**The gate, green** (cargo-deny 0.20.2 installed, advisory database fetched):

```
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
advisories ok, bans ok, licenses ok, sources ok
gate: fmt ok
gate: clippy ok
gate: test ok
gate: deny ok
gate: all steps green
```

cargo-deny also warns `license-not-encountered` for 11 of the 12 allowlisted licenses.
That is expected while the graph holds no dependency, and it is a warning, not a
refusal.

**The deny config, refuted** in a scratch copy of the workspace under `%TEMP%` (never
committed), one plant at a time:

```
== plant A: an MPL-2.0 crate (option-ext 0.2)
error[rejected]: failed to satisfy license requirements
   │            rejected: license is not explicitly allowed
licenses FAILED        (exit 4)
== plant B: the same crate at version "*"
error[wildcard]: found 1 wildcard dependency for crate 'xtask'.
bans FAILED            (exit 2)
```

**The `--locked` claim, refuted and then fixed.** In a scratch copy, `tempfile = "3"`
(an approved crate) was added to `xtask/Cargo.toml` without updating `Cargo.lock`, and
`cargo xtask gate` was run under each alias:

```
== old alias: xtask = "run --quiet --package xtask --"
gate: all steps green
rc=0 lock before=f1997875b203 after=59066b9db82b
== new alias: xtask = "run --quiet --locked --package xtask --"
error: cannot update the lock file …Cargo.lock because --locked was passed to prevent this
rc=101 lock before=f1997875b203 after=f1997875b203
```

The old alias rewrote the lockfile and reported green. The fixed one refuses, and the
lockfile is unchanged. CI must start the gate through the alias, or through
`cargo run --locked`, too (R0.2).

**The mutation round** (cargo-mutants 27.1.0, run from the worktree; it mutates a copy
of the tree in `%TEMP%`, writes its report to `%TEMP%`, and `CARGO_TARGET_DIR` is unset
so each copy builds in its own target):

```
ok       Unmutated baseline in 2s build + 4s test
31 mutants tested in 2m: 26 caught, 5 unviable
```

The five unviable mutants do not compile: `StepResult` and `Command` have no `Default`,
and `||` cannot join a `let` chain.

**The control.** A round of 31 caught out of 31 is only evidence if the harness can
report a miss (memory: *a mutation round needs a control*). The same round with only the
unit tests (`--cargo-test-arg=--lib`) reports **12 missed, 14 caught, 5 unviable**. The
12 are exactly the mutants that only `tests/cli.rs` catches: `dispatch`, both `main`s,
`workspace_root`, and the eight return-value mutants of the real `cargo` and
`cargo_silent`. Among them is `gate::main → Default::default()`, a gate that reports
success without running a step. So the harness reports survivors, and the integration
tests are what kill the gate's most dangerous mutant.

The SHA-256 of every `xtask` source file matched before and after both rounds, and
`git status` showed nothing the rounds left behind.

**Not proven:** Linux and macOS, since the `rust` CI job is R0.2. Also unproven: the
gate's `DidNotFinish` path from a real cargo killed by a signal. The real runner's `None`
is proved for a binary that cannot start, and the signal case is covered by the scripted
runner only.

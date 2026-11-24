# R2.2 engine adapters

**Status** in progress on `feature/r2-2-eph-engines`.

## Contract

[SDD-RUST §3](../sdd/SDD-RUST.md#3-the-terminal-plane) was approved whole by the
Architect on 2026-11-24 before adapter code. [ADR-0044](../adr/ADR-0044-bound-engine-probes-and-regenerate-private-settings.md)
records approval for bounded version and authentication probes and the
regeneration of private settings. The follow-up decisions cover offline fixture
replay, vetted dependencies and the prompt-literal guard.

## Evidence to date

- **Tier A.** On Windows 10.0.26300, real version commands returned Claude
  2.1.295, Codex 0.171.0-alpha.13 and Gemini 0.57.0. The same Windows session
  recorded two completed, logged-out responses from Claude. The
  [capture provenance](../../crates/eph-engines/tests/fixtures/engine-output/PROVENANCE.json)
  carries the commands, exit codes, launch conditions and substitutions.
- **Tier A.** On Windows with Rust 1.96.0 and the target in LocalAppData,
  `cargo test --locked -p eph-engines -p xtask --quiet` passed 17
  `eph-engines` tests and 136 `xtask` tests. The fake adapter drove a process
  through the real PTY. The public engine probe methods classified recorded
  bytes emitted by a test process, including the logged-out result with exit 1.
  `cargo clippy --locked -p eph-engines -p xtask --all-targets -- -D warnings`
  also exited 0 on that worktree.
- **Tier A.** On the same Windows toolchain, `cargo xtask invariants` exited 1
  for each planted literal in the actual renderer and sender. It reported
  `[prompt-literal]` at `prompt.rs:41` and `adapter.rs:519` respectively.
  The command exited 0 after the files were restored.
- **Tier A.** The first full `cargo xtask gate` passed through dependency
  checks and exited 1 at coverage. The new crate had no floor.
  `xtask` measured 97.86% against 98.29%, prompting cases for uncovered
  prompt-guard refusal branches.
- **Tier A.** On the second full gate, `xtask` measured 98.52% (2,399 of
  2,435 lines) and `eph-engines` measured 92.40% (815 of 882 lines).
  The gate's sole failure was the new crate's missing floor. The floors file
  was unchanged.
- **Tier A.** The preliminary `cargo xtask mutants xtask` found 654 mutants
  and passed its unmutated baseline. The run was interrupted after the
  follow-up refutation found an indexed argument replacement that warranted a
  source fix. It produced no completed round or control verdict.
- **Tier A.** The real invariants command then rejected the planted indexed
  write at `adapter.rs:485`. After restoration, the full gate again reached
  coverage with one failure, the missing `eph-engines` floor. That run measured
  `xtask` at 98.52% (2,405 of 2,441 lines) and `eph-engines` at 92.63%
  (817 of 882 lines).
- **Tier A.** The preliminary mutation tool copied 2.22 GB. A compact host
  Temp copy held 1,125 tracked and non-ignored files (16.2 MB), verified
  byte-for-byte against the worktree at capture time. From that copy,
  `cargo-mutants mutants --list --package xtask` exited 0 and listed 658
  candidates. This was a worktree snapshot and candidate inventory. The final
  round and control still owe results on the chosen source.
- **Tier A.** Windows `cargo xtask mutants xtask` on the compact snapshot
  passed the baseline, caught 602 of 658 candidates, missed 11 and reported
  45 that did not build. The round had zero timeouts. The no-tests control
  missed 148 of 149 viable mutants and timed out once. Its timeout was in
  the test phase, with 16,528 seconds of elapsed time. The harness compared
  72 files with no changes and exited 3 (`INVALID`). This run supplies the
  survivor list. It does not satisfy the mutation criterion. Windows power
  events record sleep and wake during the timeout log's creation-to-write
  interval.
- **Tier A.** The survivor list prompted tests for workspace scope, inline
  modules, exact prompt forwarding syntax and ordinary assignments.
  `cargo fmt --all -- --check` and
  `cargo test --locked -p xtask -p eph-engines --quiet` exited 0 on Windows
  after the test additions. The latter passed 17 engine and 140 xtask tests.
- **Tier A.** A filtered Windows `cargo mutants` replay over `xtask` matched
  the 11 survivor names exactly, passed its unmutated baseline and reported
  `11 mutants tested in 2m: 11 caught`. It ran on a new Temp copy of 1,125
  files, verified byte-for-byte at capture.
- **Tier A.** The next Windows `cargo xtask mutants xtask` exited 0
  and reported `ROUND OK` (report `ephesus-mutants-xtask-132092`).
  Its baseline passed. Of 658 cases, 612 were caught, 46 failed to build,
  none survived, and none timed out. The control missed all 149 viable
  cases, with 16 build failures and zero timeouts. The harness reported
  72 unchanged files. One round build failed with linker error LNK1104.
  In a focused replay against the same snapshot, that mutant built and
  was caught. The replay also caught the coverage case selected by the
  checked-in spec.
- **Tier A.** The settings installer replaced an ordinary hard-linked
  `eph-settings.json` and preserved the exact bytes under the other name.
  The regression passed with the full engine suite on Windows (18 tests)
  and on Ubuntu under WSL2 (19 tests). Targeted Clippy exited 0 on both
  systems, with Rust 1.96.0. The WSL2 worktree copy was checked against
  1,125 SHA-256 file hashes.
- **Tier A.** On WSL2 a real owned descendant closed both probe pipes and
  kept a separate Unix socket open. The probe preserved the root's exit 17
  and the test observed socket EOF. A plant that omitted the shutdown call
  left the peer connected and failed at the read with `WouldBlock`. The
  source hash check and the test passed after restoration. Native macOS
  behavior is still owed.
- **Tier A.** The Windows gate passed format, Clippy, tests, audits,
  invariants and dependency checks after the two review decisions.
  It exited 1 only because `eph-engines` has no floor. The new crate
  measured 92.32% (818 of 886 lines). `xtask` measured 98.52% above its
  98.29% floor. `node scripts/check-attribution.cjs` exited 0.
- **Tier A.** The first Windows `cargo xtask mutants eph-engines` exited 3
  (`ephesus-mutants-eph-engines-53148`). The baseline passed. The round caught
  145 of 243 cases, missed 26 and had 72 build failures and zero timeouts.
  The control missed 44 of 57 viable cases and had 13 timeouts with the
  automatic 20-second test limit. The harness compared 72 unchanged files.
  This run gives the survivor inventory and fails the control criterion.
- **Tier A.** Tests added from that inventory passed on Windows (24 cases)
  and WSL2 Linux (25 cases), with targeted Clippy green on both. A real
  junction on Windows and a symlink on Linux exercised the private
  directory and tool-root checks. The Linux snapshot matched all 1,125
  source file hashes before testing.
- **Tier A.** Four Windows-inactive mutations were planted independently on
  WSL2. The environment equality change failed conformance. The three
  changes to POSIX probe cleanup and root observation each failed the
  descendant-cleanup test. Each planted case passed its zero-tests
  control. After restoration all 1,125 hashes matched and the 25-case
  engine suite passed again.
- **Tier A.** A focused Windows replay selected 18 exact names from the
  survivor report. It caught 17 and missed the version-parser mutant that
  accepted a malformed numeric component. After adding empty and
  nonnumeric component cases, a one-mutant replay caught that survivor.
  Its zero-tests control missed it, and both runs had zero timeouts.
- **Tier A.** On WSL2 with Rust 1.96.0 and cargo-llvm-cov 0.9.1,
  `cargo llvm-cov --locked -p eph-engines` exited 0. All 25 engine
  tests passed. The tool's own report, which includes source test lines,
  measured 1,174 of 1,308 lines (89.76%). All 1,125 recorded worktree
  hashes matched before and after. The repeated runs against the
  committed tree and CI floor measurements remain owed.
- **Tier A.** After the mutation follow-up decisions, Windows passed all
  27 engine tests and targeted Clippy. WSL2 passed all 28 engine tests
  both plain and instrumented, plus targeted Clippy. The new tests check
  both shutdown observations, the 500 ms cleanup deadline and the
  distinction between a missing grant and a failed metadata inspection.
  The four Windows-inactive plants were caught again on the updated
  WSL2 snapshot and missed by their zero-tests controls. All 1,125
  recorded source hashes matched after the plants were restored.
- **Tier A.** Windows `cargo xtask mutants eph-engines` ran with the
  approved 90-second launcher minimum (`ephesus-mutants-eph-engines-42596`).
  Both baselines passed. The round listed 169 caught, three missed and 72
  build failures out of 244 cases. The no-tests control missed all 57
  viable cases, with four build failures. Neither phase timed out.
  The harness compared 72 unchanged files and exited 1 with
  `3 SURVIVOR(S)`. All three change Unix-only code and are caught by
  the WSL2 plants on the same production source.
- **Tier A.** Two entries reported as caught on Windows failed in
  unrelated probe fixtures. The Unix-only shutdown guard was caught by
  its WSL2 plant. A focused replay on Windows caught the Claude
  credential-directory mutant at the exact conformance assertion and
  missed it with zero tests. Both targeted phases had zero timeouts.
  The evidence accounts for 168 Windows catches, four WSL2 catches for
  cases inactive on Windows, and 72 mutation build failures.
- **Tier A.** After the final review assertions, Windows
  `cargo xtask gate` passed formatting, Clippy, tests, audits, invariants
  and dependencies. The only failure was the missing `eph-engines` floor.
  It measured 95.58% (865 of 905 lines). `xtask` measured 98.52% with a
  98.29% floor. A comparison against the mutation snapshot found
  byte-identical production engine sources. The only source differences
  were in the test modules of `grants.rs` and `probe.rs`.

## Read-only checks

The advisor distinguished R2's event declarations and adapter contract from
R3's live hook delivery and R6's transcript reader. The design review and the
refutation pass identified raw prompt arguments inside the checked sender,
relative or mismatched engine config directories, optional helper aliases that
could survive an omitted grant and conformance checks that accepted incorrect
CLI prefixes or permission modes. The worktree includes fixes and tests for
those findings. The process test also confirms the descendant writes its
readiness marker before the probe reports a deadline. It does not independently
observe the root exit. The follow-up refutation found an indexed argument write
that could replace the sent prompt. The guard now checks that assignment target.
The latest review compared the shutdown and grant-inspection seams with the
prior production code. It found the native termination, polling and error
order preserved. The tests now exercise the dangling link through the public
resolver and check the last one-millisecond shutdown pause. A real failure
to schedule detached Unix reaping still has no fixture.

## Pending

- Finish the repeated checks against the committed tree. The green
  workspace gate, cross-platform CI and floor ratchet follow a separately
  approved push.
- R2.3 integration and the signed-in reference-engine run at R2 exit.

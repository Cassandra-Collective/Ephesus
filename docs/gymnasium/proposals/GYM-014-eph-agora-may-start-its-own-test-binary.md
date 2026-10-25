# GYM-014 — `eph-agora` may start a process, so a kill mid-append is a real kill

**Status:** landed (R1.4b, 2026-10-25; metrics 1 and 2 measured at its close, metric 3 due
at R2's close, in the ledger) · **Proposed:** 2026-10-25 · **Decided:** 2026-10-25 (Architect,
asked before any R1.4b code: a real kill as well as the planted prefixes, with this
row) ·
**Gate:** Architect approval. It changes what `cargo xtask invariants`, a step of the
`rust` CI job, refuses: one more crate may claim the `process:` exemption. That crate is
not in RUST-PLAN §3.2's list of the crates that may start a process. It touches no ADR,
no secret and no dependency.

**Filed before the code**, as the 2026-09-02 ruling asks of any change to what a gate
refuses (DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, second round*, item 7).
It is its own row, not part of the umbrella GYM-013, because GYM-013 measures the
rewrite's reason and its gate's planted failures, and this widens who the gate allows.

---

## Evidence

RUST-PLAN §5 makes R1's exit "killed mid-append and mid-commit, nothing is lost or torn
on reopen". Mid-commit is proved by real kills: `eph-git`'s
`a_harness_killed_at_each_point_of_a_commit_loses_nothing` and
`a_harness_killed_at_random_during_commits_loses_nothing` rerun the test binary as the
harness and kill it. They can because `xtask`'s policy table lets `eph-git` claim
`process:`, which only its tests use (DECISIONS-LOG 2026-10-20, R1.3 implementation,
item 18).

Mid-append has never been proved by a kill. `eph-fs`'s append tests plant a torn line by
hand. An append is one synced `write_all` of `[\n]line\n`, so a kill can leave only a
prefix of those bytes, and R1.4b plants every such prefix. A plant shows that the reader
copes with each state a kill can leave. It cannot show that a real kill leaves only
those states, or that the writer's next open, the boot's `agora/log-torn` and the
reader agree on a file a dying process actually wrote.

`eph-agora` holds the book of record, so the kill test belongs in its suite. Its policy
is `xtask`'s default, which grants no exemption, so the test cannot start the child it
must kill.

## Proposal

1. `xtask/src/invariants.rs`'s policy table names `eph-agora` with
   `exemptions: &[Category::Process]`, as it names `eph-git` with git and process.
2. `eph-agora`'s tests rerun the test binary as a child that appends to a real book in a
   loop, and kill it: at random moments and at each fault point the writer offers.
   After each kill, a reopen must read every line the child reported as acknowledged,
   skip at most one torn tail, raise `agora/log-torn` exactly when the live file ends
   mid-line, and append the next line whole.
3. RUST-PLAN §3.2 gains a sentence: `eph-agora` claims `process:` for its kill tests
   only, as `eph-git` does.

## Cost & risk

The exemption is per crate, not per test: production code in `eph-agora` could claim
`process:` too, with a reason, and `invariants` would pass it. Review is the guard, as it
is for `eph-git`, and metric 3 measures it. A per-test category would close that, but it
is a larger change to the gate and was not asked for.

A kill test can flake on a slow machine. The memories *a race-sampling test passes under
its own condition* and *a fork holds every fd until exec* say how, so the test is
measured plain and instrumented, on win32 and Linux, before R1.4b closes.

## Success metric

Measured at R1.4b's close, and again at R2's close:

1. **The gate still refuses what it refused.** A planted `std::process::Command::new`
   under a `process:` reason passes `invariants` in `eph-agora` and still fails it by
   file and line in `eph-schema` and `eph-fs` (refuted on a scratch copy, as R0.2's
   rules were).
2. **The kill is real and nothing is lost.** The kill test passes in at least 10 plain
   and 10 instrumented (`cargo llvm-cov --workspace --no-report`) runs each on win32 and
   in WSL2 Linux, and on CI's three platforms, with every acknowledged line read back
   after every kill.
3. **The exemption stays in the tests.** At R2's close, no `process:` exemption in
   `eph-agora` sits outside an item compiled only for tests (`#[cfg(test)]`).

If metric 3 fails, the row has regressed.

## Rollback

Remove `eph-agora` from the policy table and delete the kill test. The planted-prefix
test stays, and R1's exit then rests on it alone, recorded as such.

## Related

- DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions, second round*, item 7
- RUST-PLAN §3.2 and §5 (R1's exit)
- [GYM-013](./GYM-013-the-rust-rewrite-is-measured.md), the rewrite's umbrella row

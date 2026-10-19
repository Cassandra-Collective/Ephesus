# GYM-013 — The Rust rewrite is measured: "faster" is a number, not a claim

**Status:** proposed · **Proposed:** 2026-10-19 · **Decided:** — ·
**Gate:** Architect approval. It replaces the whole gate chain, touches every
BUILD-PROMPT §3 invariant's enforcement, supersedes an ADR and adds dependencies, so it
is the ADR-0015 class that also asks for a decision memo. The memo is
[`docs/RUST-CRATES.md`](../../RUST-CRATES.md). The decision is
[ADR-0041](../../adr/ADR-0041-ephesus-is-rewritten-in-rust.md).

**Filed before the code**, as the 2026-09-02 ruling asks of any change to what a gate
refuses. The Architect chose this umbrella row on 2026-10-19 over recording the rewrite
in DECISIONS-LOG and ADR-0041 alone. The id is 013 because GYM-012 is filed on
`fix/gym-008-follow-up`, which is not yet merged.

**This row is unusual in one way.** The change is the Architect's direction, not an
improvement the company proposed, so the row does not ask whether to rewrite. It asks
him to approve what the rewrite will be measured against, and so whether his reason for
it, a faster and lighter harness, turned out true.

---

## Evidence

The Architect's direction of 2026-10-19: the TypeScript build had grown slow to extend
and heavy to run. Measured on that build before it was frozen (win32, Electron 37,
built app, fresh scratch home, consent not granted so nothing spawned, 30 s idle; the
method is in `docs/implementations/2026-10-19-rust-rewrite-direction.md` §6):

| Measure | Cold run | Warm runs | Baseline used |
|---|---|---|---|
| Process start → control surface serving | 11.06 s | 1.50 s, 1.55 s | **1.53 s** (warm mean) |
| Private bytes, whole process tree | 281.0 MB | 269.6 MB, 269.4 MB | **269.5 MB** (warm mean) |
| Working set, whole process tree | 434.6 MB | 520.8 MB, 523.5 MB | 522.2 MB (reported, not a metric) |
| Processes | 4 | 4, 4 | 4 |
| On disk to run it from a build | — | — | **705 MB** (Electron 318 + app 4 + runtime deps 383) |

## Proposal

The rewrite as ADR-0041 and `docs/RUST-PLAN.md` define it. The gate chain becomes
`cargo xtask gate` (RUST-PLAN §4), run in a new `rust` CI job on Linux and Windows. The
TypeScript jobs keep running over the frozen tree until the Architect switches `main`'s
required checks. `check-readme-current.cjs` learns the `R` package prefix.

## Cost & risk

Every component of the system, phases R0 to R14. The blast radius is the whole product.
Measured risks are in RUST-PLAN §8, and the highest is terminal fidelity. A
regression in *capability* is guarded by ADR-0041's parity definition, not by this row.
This row guards the reason the rewrite exists.

## Success metric

Measured on the same machine as the baseline, by the same method (same scratch-home
boot, same 30 s idle, three runs with the first reported as cold), **within two weeks of
R14 landing**. The thresholds are proposed for the Architect to approve or amend:

1. **Startup.** Warm mean time from process start to the control surface serving is
   **≤ 0.75 s** (≤ 50 % of 1.53 s), with the window open.
2. **Memory.** Warm mean private bytes of the process tree, 30 s after start with the
   window open on the roster, is **≤ 135 MB** (≤ 50 % of 269.5 MB).
3. **Footprint.** What it takes to run the Rust build from a release build (binaries
   plus bundled assets, excluding the licensed art pack, which the TypeScript figure
   also excluded) is **≤ 70 MB** (≤ 10 % of 705 MB).
4. **The gate is real.** Every `xtask` rule fails a planted violation by file and line
   at the phase that adds it, and at R14 the `rust` job is green on Linux and Windows on
   the deletion commit.

**Checkpoint, not a metric:** at R7 (the headless company) metrics 1 and 2 are measured
on `ephesusd` and reported, so a miss shows long before R14. A checkpoint cannot pass or
fail this row.

## Rollback

Until R14 the TypeScript tree is in place and its CI still guards `main`, so nothing is
lost while the rewrite runs. If a metric fails at R14, the deletion commit does not land
until the Architect rules: amend the threshold on the record, fix the cause, or keep the
TypeScript build. The ledger records the outcome either way. A regressed row is training
data, not a row to delete.

# The record after R2.1 merged: six stale records corrected

**Date:** 2026-11-23 · **Branch:** `docs/r2-1-record-gaps`, cut from `origin/main` at
`45dcff9` · **Scope:** documentation and one reason string. No code, no TypeScript file,
no ADR, no floor. · **Decisions:** DECISIONS-LOG, 2026-11-23, *Architect decisions on the
record*, items 1 to 4 · **Found by:** the [status snapshot of 2026-11-23](../status/2026-11-23-status.md).

## 1. Problem / motivation

On 2026-11-23 the whole Rust stack merged to `main`: #105 to #112, with R2.1 (#107) last
as `45dcff9`. Several records still described the stack as it was before it merged. A
session that resumes from them would be misdirected, and the goal handover is the worst
case.

- **`RUST-GOAL.md`** said R2.1 had not started and told the session to check out
  `feature/r1-4b-book-of-record`. Its prompt tells a session to resume at that file's
  next step, so the next `/goal` run would have restarted R2.1's probe after R2.1 had
  merged.
- **PROGRESS's resume line** read "As of 2026-10-25 … R0.3's is next".
- **PROGRESS's R0 section** still listed #104 as open.
- **The R2.1 tick** was the only R tick without a dated evidence note.
- **SDD-RUST's status line** recorded §2, §4 and §10 as approved and said nothing on §3.
  Its section table gave §3, "the engine adapter surface", to R2, although §3 holds only
  R2.1's terminal half.
- **`xtask/coverage-floors.json`'s `toleranceReason`** still said "No crate yet has a
  timing-dependent path". Its own rule was that such a crate re-measures and records its
  spread there, and R2.1 had not done so.
- **`fix/worktree-untracked-files`**, the TypeScript data-loss fix, was decided on
  2026-10-20 as a PR outside the stack. It is pushed and was never opened. The record
  already listed it as waiting on the Architect, so this was an open action, not a wrong
  record.

## 2. What changed

| File | Change |
|---|---|
| `docs/DECISIONS-LOG.md` | One entry appended with the Architect's four answers. |
| `docs/PROGRESS.md` | The resume line now says everything through R2.1 is merged and R2.2 is next. The fragment around the integration CI link is mended, with #107's merge and its CI run added. The R2.1 tick has a dated evidence note, and R0's merge line names #104 and #105. |
| `docs/RUST-GOAL.md` | The branch to check out is now R2.2's. The state table is current and an R2.1 close summary replaces R1.4b's. *Next* is rewritten for R2.2, R2.3 and R2's exit. Settled entries are added for R2.1 and this fix. R0.3's PR item is gone, a main-checkout pull item is added, and the TypeScript fix PR is still owed. The prompt block is unchanged. |
| `docs/sdd/SDD-RUST.md` | The status line says §3 is written but not yet approved, and that it is approved whole at R2.2. The §3 table row names both halves and the package that writes each one. |
| `xtask/coverage-floors.json` | `toleranceReason` records the CI spread of the three timing-dependent crates. No row, floor or tolerance changed. |
| `docs/status/2026-11-23-status.md` | The status snapshot that found these gaps (new). |
| `docs/implementations/2026-11-23-r2-1-record-gaps.md` | This record (new). |

## 3. Implementation approach

Each stale statement was checked against a repository or CI fact before it was rewritten,
and each rewrite cites that fact:

- **Merges and SHAs:** `gh pr list --state merged`. For example, #105 merged as `dc5b6ea`,
  #104 as `9aa30bd` and #107 as `45dcff9`.
- **CI conclusions:** `gh pr view 107` and `gh run list --branch main`. Runs 45196366436
  and 45197694980 are green on `45dcff9`.
- **The coverage spread:** read from the logs of CI run 45122195829 (section 4).
- **The TypeScript reference files named in RUST-GOAL's next step:** listed from
  `src/main/engines/` and `test/main/engines/`.

Nothing was reconciled silently. Where a record and the facts disagreed and more than one
fix was possible, the Architect chose, through AskUserQuestion, before the edit. That
covered the floors file, SDD-RUST §3's approval, the TypeScript PR and R2.2's branch.

The goal prompt block was left byte for byte as it was. Only the prose around it changed,
so the CLI's 4,000-character limit on the condition was not touched.

## 4. Mathematical / statistical details

The floors file sets `tolerance` to 0.00. A tolerance of zero is only honest if a tree
measures the same on every attempt. The spread of crate *c* on platform *p* over attempts
*a* = 1..3 of one CI run is

  spread(*c*, *p*) = max_a cov(*c*, *p*, *a*) − min_a cov(*c*, *p*, *a*),

where cov is production-line coverage in percent: covered lines ÷ production lines × 100.

The figures below were read from each attempt's `coverage:` lines in the logs of CI run
45122195829, on tree `fnv1a64:24b271b44e7dc27b`:

| Crate | Linux | macOS | Windows | Spread on each platform |
|---|---|---|---|---|
| `eph-process` | 167/176 = 94.88% | 216/227 = 95.15% | 379/396 = 95.70% | 0.00 |
| `eph-pty` | 307/323 = 95.04% | 307/323 = 95.04% | 319/336 = 94.94% | 0.00 |
| `eph-git` | 1124/1129 = 99.55% | 1124/1129 = 99.55% | 1133/1137 = 99.64% | 0.00 |

All three attempts reported the same line counts, not just the same rounded percentages.
So the zero tolerance still holds for the crates whose paths depend on timing.

The differences between platforms are not spread. They come from platform-specific
code, which is what each crate's per-platform figure records.

## 5. Design decisions

- **The tolerance reason was hand-edited (Architect, item 2).** `cargo xtask coverage
  ratchet` copies the reason through unchanged. No ratchet run can correct it, so the
  only alternatives were to leave the stale sentence or to change xtask. Rejected:
  recording the spread elsewhere and leaving the sentence false; teaching the ratchet to
  write the reason, which would be an xtask package with its own mutation round. The
  committed-file test `the_committed_floors_file_reads_and_renders_back_unchanged`
  guards the exception: the file must still parse and render back byte for byte.
- **SDD-RUST §3 is approved whole at R2.2 (Architect, item 3).** One approval covers the
  section once both halves exist, the way R1.0 approved §2 and §4 together. Rejected:
  approving the terminal half now. That would split one section's approval across two
  dates and two states of the text.
- **R2.2 is cut from `main` (Architect, item 1).** The stacking rule cut each package
  from the previous package's tip so that unmerged work stayed reachable. R2.1's tip is
  merged, and `main` holds it plus the merge, so stacking no longer buys anything.
- **The TypeScript fix PR stays owed (Architect, item 4).** Opening a PR is outward-facing
  and is the Architect's step. RUST-GOAL keeps the `gh` command and now says to
  trial-merge first, because `main` has moved since 2026-10-20.
- **SDD-RUST's §3 heading was not renamed.** `#3-the-terminal-plane` is linked from
  RUST-PLAN §8 and from DECISIONS-LOG, which is append-only. Renaming the heading would
  break those anchors. The section table names both halves instead.
- **R1's verdict text was left as written.** Its "Open for the Architect" list still names
  R0.3's PR. It is a dated verdict, and rewriting it would change history. The live
  status lines around it were updated instead.

## 6. Verification

Run from the repository root on this branch:

1. The floors file still reads and renders back byte for byte:

   ```bash
   CARGO_TARGET_DIR="$LOCALAPPDATA/ephesus-cargo-target" cargo test -p xtask --locked the_committed_floors_file_reads_and_renders_back_unchanged
   ```

   Observed on win32 on 2026-11-23: `test result: ok. 1 passed; 0 failed`.
2. Every relative Markdown link resolves. CI's docs-integrity loop, run repo-wide with
   `node_modules` excluded, exited 0.
3. `node scripts/check-readme-current.cjs` reported "README landed list is current for
   R2 (1 package(s))" and exited 0.
4. The goal condition is 3,964 characters, under the 4,000 limit, and no line of the
   prompt block changed. Measure it with the command in RUST-GOAL's *Its length*.
5. The spread in section 4 can be re-read with:

   ```bash
   gh run view 45122195829 --attempt <n> --log | grep -E "coverage: eph-(git|process|pty):"
   ```

   Run it for attempts 1, 2 and 3.

Not run: `cargo xtask gate`, because no Rust source changed and the floors file is read
only by the coverage step and the round-trip test. Not run: the TypeScript checks, because
no TypeScript file changed. CI runs both on a PR.

## 7. Related docs

- [Status snapshot, 2026-11-23](../status/2026-11-23-status.md)
- [PROGRESS](../PROGRESS.md), *Resume here* and *The Rust rewrite*
- [RUST-GOAL](../RUST-GOAL.md)
- [RUST-PLAN](../RUST-PLAN.md) §3, §5, §6 and §9
- [SDD-RUST](../sdd/SDD-RUST.md) §3
- [R2.1 proof record](./2026-11-20-r2-1-guard-proof.md)
- [DECISIONS-LOG](../DECISIONS-LOG.md), 2026-11-23

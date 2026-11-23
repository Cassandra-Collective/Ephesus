# R0.3 — The standards, the requirements, the README check and the skills learn Rust

**Date:** 2026-10-19 · **Branch:** `feature/r0-3-standards-learn-rust` (stacked on
`feature/r0-2-gate-chain`) · **Package:** R0.3 (`docs/RUST-PLAN.md` §5) ·
**Decisions:** DECISIONS-LOG 2026-10-19 (R0.3: the README, the Rust gate, the
website; the SRS wording approved with the plan) · **Gymnasium:** GYM-013, under
whose Proposal this check's change falls

## 1. Problem / motivation

After R0.2 the Rust workspace had a gate, and the documents a session reads still
described only the TypeScript build:

- ENGINEERING-STANDARDS said "TypeScript everywhere", and its Definition of Done named
  only the `npm` gate.
- BUILD-PROMPT's TEST line and build order did not mention the rewrite.
- `/goal` and `/build-package` would have run the full `npm` suite before every Rust
  commit, and cut branches named for M packages.
- The SRS still named node-pty, xterm.js, `contextIsolation` and TypeScript (FR-1.1,
  NFR-8, NFR-16).
- `check-readme-current.cjs` parsed only `M`-prefixed package ids. R0.1 and R0.2 were
  ticked, the README said nothing about the rewrite, and the check passed. That is the
  failure the check exists to catch, hidden by the check's own regex.

## 2. What changed

| File | Change |
|---|---|
| `docs/ENGINEERING-STANDARDS.md` | §1.1, the Rust workspace's operational rules: toolchain, the gate, lints and exemptions, dependencies, mutation rounds, the pre-commit gate for a Rust package. The invariants are linked to ADR-0041's table, not restated. DoD items 1 and 9 gain their Rust form, and item 7's coverage half is dated to R1. |
| `BUILD-PROMPT.md` | §3 points at the Rust form of the invariants. §4's TEST line gains the Rust line. §5 puts R0 → R14 between M8c and M9. |
| `docs/srs/SRS.md` | FR-1.1, NFR-8 and NFR-16 amended to the approved wording (RUST-PLAN §10), dated, with the old wording preserved. |
| `scripts/check-readme-current.cjs` | The tick regex reads the `R` track. A milestone is keyed by letter and number (`R0` ≠ `M0`). The summary-row exclusion holds for both tracks. |
| `test/scripts/check-readme-current.test.ts` | Four R-track cases, each built so one wrong reading gives a different verdict. The real-repository case admits `[MR]\d+`. |
| `test/mutation/readme-current.json` | Three mutants for the new rules (`R1`–`R3`) beside the nine existing ones and their control. |
| `docs/PROGRESS.md` | The R section moved to its build-order place, after M8c and before M9 (the check reads "most recent" in register order). R0.3 ticked. |
| `README.md` | Status says the rewrite is under way and what still runs today. "Where the build stands" opens on the rewrite, with the marker `R0.1 R0.2 R0.3` and a paragraph each. The M8 marker is kept as a plain comment. |
| `.claude/skills/{goal,build-package,milestone-review,ephesus-engineer}/SKILL.md` | The R order, the `feature/r<phase>-<n>-<slug>` branches, the Rust gate and the Rust exit criteria. |
| `docs/DECISIONS-LOG.md` | The R0.3 decisions. |

## 3. Implementation approach

**Link, do not restate.** CLAUDE.md's rule is that a requirement lives in one place.
ADR-0041's table says how each invariant holds in Rust, so ENGINEERING-STANDARDS §1.1,
BUILD-PROMPT §3 and the engineer skill each point at it. They add only what the table
does not carry: the operational rules R0.1 and R0.2 paid for. Those are the `--locked`
alias, the category-tagged `#[expect]`, fixtures kept out of `mutants.out/` and
`target/`, and the frozen tree's lint over every Rust diff.

**The README check's design held, and needed one fact made true.** The check's rule,
"the README describes the milestone holding the most recently ticked package", is
right for two tracks. Two things had to change for it. The regex had to see `R`. And
"most recent" is register order, so the R section had to sit where the rewrite falls
in the build order. It sat above M8b, where every M8c tick would have read as newer
than R0.3. The summary-row rule (`**M8**`, `**R1**` are not packages) now compares the
id with the whole key instead of rebuilding an `M` prefix.

**Archiving an R phase is not built.** The archive accepts `M<n>[letter].md` and sorts
by number. An R phase archived as `R0.md` would need a cross-track order (M8c → R0 …
R14 → M9), and nothing needs one until R0 closes. It is recorded here and in the
script's comment instead of built ahead of its use.

## 4. Mathematical / statistical details

Not applicable: a structural change.

## 5. Design decisions

- **The README marker names R0 only.** The check scopes the README to one milestone
  on purpose ("Landed so far" is the current milestone's narrative), so M8's eighteen
  names leave the marker and stay as a comment. Listing both would mean writing a
  two-track check to serve one README.
- **The website was not touched** (Architect, 2026-10-19), until R4.
- **The real-repository test admits `[MR]\d+` rather than pinning `R0`.** A test that
  pins the current phase breaks on the next tick for no defect.

## 6. Verification

Win32, Node v24.20.0, vitest 4.1.11 through the main checkout's `node_modules`
(junctioned; `package-lock.json` unchanged since the 353-of-353 check). Tier A.

**The check, on the real tree, before the README learned the rewrite.** With the regex
taught `R` and the R section moved, `node scripts/check-readme-current.cjs` went red,
naming exactly what was missing:

```
  ticked in the register, absent from the README: R0.1, R0.2
  claimed by the README, not ticked in the register: M8.0, M8.1, … M8b, M8c
```

With the README's R0 marker written and R0.3 still unticked, it named the one claim
made too early: `claimed by the README, not ticked in the register: R0.3`. With R0.3
ticked: `README landed list is current for R0 (3 package(s)).`

**The tests:** `npx vitest run test/scripts/check-readme-current.test.ts`, 16 passed
(16). The four R-track cases are: R as the milestone when ticked last; `R0` is not
`M0`; an R phase's summary row is not a package; an M milestone read as before,
summary row included.

**The frozen tree's gate over this diff:** `prettier --check .` clean, `eslint .
--max-warnings 0` exit 0, `tsc --noEmit -p tsconfig.node.json` exit 0,
`check-invariants`, `check-attribution`. The Rust gate is unchanged by this package
and green.

**The mutation round** (GYM-008's tool, DoD item 9). It ran from the committed spec in
a detached scratch worktree of `efda5e6` under `%TEMP%`, outside OneDrive, with
`node_modules` junctioned in. Free RAM was 2.66 GB. Verbatim:

```
anchors: 12 of 12 match exactly once
baseline: PASS — 1 file, 16 passed: test/scripts/check-readme-current.test.ts
  control-comment  control  SURVIVED — certifies the round  [1 file, 16 passed]
  A1-archive-ignored  killed — 3 tests failed  [1 file]
  A2-progress-read-first  killed — 2 tests failed  [1 file]
  A3-string-sort  killed — 1 tests failed  [1 file]
  A4-stray-skipped  killed — 1 tests failed  [1 file]
  A5-no-archive-throws  killed — 9 tests failed  [1 file]
  V1-surplus-ignored  killed — 2 tests failed  [1 file]
  C1-exit-status-dropped  killed — 1 tests failed  [1 file]
  C2-reasons-unprinted  killed — 1 tests failed  [1 file]
  R1-r-track-unseen  killed — 4 tests failed  [1 file]
  R2-number-only-key  killed — 8 tests failed  [1 file]
  R3-summary-row-counted  killed — 2 tests failed  [1 file]
ROUND OK — 11 of 11 real mutants killed; the control survived.
```

The three new mutants are each killed by the case built against them, and the nine
from the archive split still are. The scratch worktree was removed after the junction,
and the donor `node_modules` held 257 entries before and after.

**Not proven here:** the skills' new text in a fresh session. Worktree sessions load
skills from the main checkout, so they take effect when this merges and the main
checkout pulls (memory: worktree sessions load skills from the main checkout).

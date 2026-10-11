# The register resumes from a block, and closed milestones move to an archive

**Date:** 2026-10-11 · **Branch:** `claude/sharp-goldberg-rivdid` (from `main` at
`c0e1534`) · **Commits:** `6009472` (the split, and the README check that reads it),
`72875d4` (every entry point names the block and M9), and the commit carrying this
document, the status snapshot and the decision entries.

---

## 1. Problem / motivation

The Architect raised three things after the status pass of 2026-10-11.

**`docs/PROGRESS.md` could not be read in one pass.** It was 7,289 lines and about
490 KB. M8's section alone was 2,085 lines. The milestones still open, M8b onward,
were about 570.

**Its resume rule no longer led anywhere.** The file said *"the next session resumes
at the first unchecked box"*. That box was M6.9, deferred indefinitely since
2026-08-30. Behind it came M7's exit, two items M7 still owes, and the exits of M8b
and M8c, all waiting on the Architect, and only then M9.0. Hiding M6.9 would not have
fixed it, and neither would any tidying of the boxes. M9.0 is itself not authorised
to start, so even a perfectly ordered list would have put a session on a box nobody
may begin. The 2026-09-15 record met the same rule from the other side, when sixteen
landed packages were missing from the register. This time every box was present and
the rule was what failed.

**M9 was not documented where a session starts.** M9 and Phase 9 have their plans,
their ADR, their SRS and SDD sections and their PROGRESS boxes. But every place a
session is told to begin pointed somewhere else:

- BUILD-PROMPT §5's strict order ended at M7b, and its newest build-state block was
  dated 2026-09-07.
- `/goal` resolved "M3, resuming at M3.1".
- `/build-package` seeded "the M0–M7 checklist".
- The README's Status section named M8 as the current milestone.

## 2. What changed

| File | Change |
|---|---|
| `docs/progress/M0.md` … `M8.md` | **New.** Ten files, each one milestone's section moved verbatim, under a five-line preamble. Only relative links changed: 26, each rebased one folder. M7's file also carries the 2026-09-06 record of the crew's first pull request (§3) |
| `docs/PROGRESS.md` | Opens with its contract, a **Resume here** block and a milestone index, then M8b onward unchanged. One sentence in the post-v1 section that taught the old rule now says only that box order is build order. 695 lines, 48 KB |
| `scripts/check-readme-current.cjs` | Reads the register as the archive in build order, then PROGRESS.md. A file in the archive not named for a milestone is refused. `check(root)` and `main()` are exported, and `main()` runs only when the file is the program |
| `test/scripts/check-readme-current.test.ts` | **New**, the script's first tests: 12 cases, two of them running the real file as CI does |
| `test/mutation/readme-current.json` | **New.** The round for the change: a control and 8 mutants |
| `.github/workflows/ci.yml` | The step's comment names the register |
| `BUILD-PROMPT.md` | §5's order runs through M9h, and says M9 onward is approved for planning only. It records the DD-M9-7 exception. A 2026-10-11 build-state block goes on top of the dated stack, and "Begin now" reads the block |
| `CLAUDE.md`, `.claude/skills/{ephesus-engineer,goal,build-package,milestone-review}/SKILL.md` | Resume from the block; update it in the commit that changes the next step; archive a closed milestone verbatim. `/goal` asks before starting a milestone the block says is not authorised |
| `README.md` | Status names M9 as current and not yet started. The documentation map gains PROGRESS.md and its archive, and IMPLEMENTATION's row says M0 through M9h. Two evidence links follow M5 and M5b into the archive |
| `docs/M9-PLAN.md`, `docs/PHASE-9-PLAN.md` | "asks first" points at the block |
| `docs/implementations/2026-08-29-m6-floor-face-and-herald.md` | Its link to `PROGRESS.md#m6` now goes to `progress/M6.md` |
| `docs/status/2026-10-11-status.md` | **New.** The snapshot, reporting movement against 2026-09-15 |
| `docs/DECISIONS-LOG.md` | The Architect's four answers, appended |

No file under `src/` changed.

## 3. Implementation approach

**The move is a script with its own proofs, not hand editing.** It splits the file
at its `## ` headings and moves M0–M8 out. The moved text changes in one way only:
each relative markdown target outside a URL or an anchor gets `../` (`./x` becomes
`../x`). It then checks two things:

- **Reversibility.** Undoing that rebase gives back every original line byte for
  byte.
- **Conservation.** The 7 old header lines + 6,660 moved + 622 kept = 7,289, the
  original count. No line is lost or duplicated.

**One block was in the wrong place, and it moved with its milestone.** The record of
the crew's first pull request (2026-09-06, SRS §6.1's core clause) had trailed the
file after M7b, behind a `---`. When #72 appended the post-v1 section, that record
ended up under the heading *"Phase 9, after v1"*. It is a record of M7's exit
criterion, so it now ends M7's archive file, behind a one-line note saying where it
came from.

**The resume block states what the record decided, not what this session would.**

- **Next step: M9.0, once the Architect says so.** M9-GOAL is the handover.
- **The three exits close on one record.** M7, M8b and M8c close on bench task T1's
  first live row, and he ticks them on it. This is **DD-M9-7**, which rejected a
  fifth manual EXIT-M8 run. The status pass earlier the same day had offered "decide
  whether to schedule the unattended run" as an open question, and it was not one.
- **M8's exit row closes on the same run.** That is what M8.12 records.
- **M6.9 stays deferred** (DD-M9-16 for speech).

**The README check could not stay as it was.** It reads ticked boxes and checks the
README's `<!-- landed: -->` list against the milestone with the most recent tick.
With M8 archived and only `PROGRESS.md` read, the most recent tick is still M8c's,
which counts as milestone 8. But M8.0–M8.14 would have been missing, so the check
fails naming sixteen packages as claimed but not ticked. That was observed before
the fix.

The register is now read the way the single file read: archived milestones by
number and then letter, then PROGRESS.md. "The most recent tick" therefore means
what it meant. The check reports `current for M8 (32 package(s))` on this tree,
exactly as before the split.

A string sort would have read M10 before M9, so the order is numeric, and a test
pins it. A stray file in the archive is refused rather than skipped, because its
ticks would otherwise drop out of the check unseen.

## 4. Mathematical / statistical details

None.

## 5. Design decisions

The four that shaped this were the Architect's, asked before anything was changed
(DECISIONS-LOG 2026-10-11):

- **Archive M0–M8.** Rejected: archiving M0–M7 only, which needs no script change
  and still leaves about 2,800 lines; and a resume block alone.
- **A Resume here block.** Rejected: keeping the box rule with deferred items moved
  out of box form. M9.0 is gated on his go-ahead, so the first box would still be
  one nobody may start.
- **Every entry point names M9.** Rejected: only PROGRESS and the snapshot, leaving
  BUILD-PROMPT as #72 deliberately had.
- **One PR, merged by the engineer once CI is green.**

Mechanical choices, per BUILD-PROMPT §8.2:

- One file per milestone, named for the milestone (`M5b.md`), so the check can read
  the archive in build order from the names alone.
- BUILD-PROMPT's older build-state blocks stay below the new one: it is a dated
  stack, newest first.
- Historical records that quote the old rule (DECISIONS-LOG, implementation docs,
  the 2026-09-15 note in PROGRESS) are left as written.
- The suffix tiebreak in the archive sort orders same-numbered files (M5, M5b). The
  verdict does not depend on it, because a milestone is its number. A mutant that
  removes the tiebreak would be killed or not depending on the order the filesystem
  lists the files, so it is not in the round: a round must not depend on luck.

## 6. Verification

Run on linux in a cloud container, Node 22.22.0, without the licensed art pack.

- **The split:** the script's own assertions, reversibility and conservation, both
  pass (§3). Tier A.
- **Every relative link** in `PROGRESS.md` and `docs/progress/`, bare ones included,
  resolves: 83 of 83. The CI loop, which checks `./` and `../` targets across the
  repository, finds 595 links and none broken. Tier A.
- **The README check:** red after the split with the old script (sixteen M8 packages
  named as claimed but not ticked), green after the change with
  `README landed list is current for M8 (32 package(s)).` Tier A.
- **The tests:** 12 of 12 pass in `test/scripts/check-readme-current.test.ts`, and the
  temp-hygiene guard is green over the new file (264 tests across the two). Tier A.
- **The mutation round,** run from `test/mutation/readme-current.json` on the
  committed tree `6009472`. Tier A:

  ```text
  node scripts/mutate.cjs test/mutation/readme-current.json
  anchors: 9 of 9 match exactly once
  baseline: PASS — 1 file, 12 passed: test/scripts/check-readme-current.test.ts
    control-comment  control  SURVIVED — certifies the round  [1 file, 12 passed]
    A1-archive-ignored  killed — 4 tests failed
    A2-progress-read-first  killed — 1 tests failed
    A3-string-sort  killed — 1 tests failed
    A4-stray-skipped  killed — 1 tests failed
    A5-no-archive-throws  killed — 5 tests failed
    V1-surplus-ignored  killed — 1 tests failed
    C1-exit-status-dropped  killed — 1 tests failed
    C2-reasons-unprinted  killed — 1 tests failed
  ROUND OK — 8 of 8 real mutants killed; the control survived.
  ```

  `C1` is the one this change had to add a test for. Every earlier case called
  `check()` directly, so a script that printed its failure and exited 0, passing
  CI, would have stayed green.
- **The gate:** `npm run typecheck`, `npm run lint` and
  `node scripts/check-invariants.cjs` are green on the branch. Tier A.

## 7. Related docs

- [`../PROGRESS.md`](../PROGRESS.md) — the block and the index.
- [`../progress/`](../progress/) — the archive.
- [`../status/2026-10-11-status.md`](../status/2026-10-11-status.md) — the snapshot
  this change was raised alongside.
- [`2026-09-15-progress-record-and-engineer-skill.md`](./2026-09-15-progress-record-and-engineer-skill.md)
  — the last time the register and its resume rule failed.
- [`../M9-GOAL.md`](../M9-GOAL.md), [`../M9-PLAN.md`](../M9-PLAN.md) — what the block
  points at.

## Not proven

- **CI on this branch.** Proven only by the PR's own run.
- **Whether a resuming session behaves as the block intends.** No session has resumed
  from it yet. The rule is prose in five places, and only the README check is
  mechanical; nothing checks that the block is current.
- **win32.** Nothing was run there, and the archive's links were checked on linux
  only (paths are case-sensitive here, so a case mismatch would have shown).

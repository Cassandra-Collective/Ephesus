# The watchlist seed validates what it parsed, and pattern intake fails closed

> **Carried to `main` on 2026-10-19 without its code.** PR #76 was closed unmerged when
> ADR-0041 froze the TypeScript tree (the Architect's direction; `docs/RUST-PLAN.md` §6).
> The code this document describes is on the closed branch `fix/stoa-seed-fail-closed`,
> not on `main`. Its decisions stand: they are requirements for the Rust Stoa (R11), and
> the DECISIONS-LOG entry of 2026-10-11 records them.

**Date:** 2026-10-11 · **Branch:** `fix/stoa-seed-fail-closed` from `origin/main`
`c0e1534` · **Decisions:** DECISIONS-LOG 2026-10-11 (Stoa seed hardening)

---

The Stoa seeds its watchlist from `docs/stoa/WATCHLIST.md` the first time a home
touches it (FR-13.7). Three defects lay along that path, and a single table cell
triggered all three. #72 fixed the cell; this change fixes the code so that the next
bad cell is reported rather than believed.

## 1. Problem / motivation

**Reproduced, not inferred.** The shipped `src/shared/stoa.ts` and `src/main/stoa.ts`
were bundled with esbuild and run over the real table: once as `main` held it at
`75ec37c`, once at #72's head `715db6b`, and once at `c0e1534` after #72 merged. A
fresh temp home was seeded each time, read, read again, and then registered one
source (win32, Node v20.16.0):

| Table at | What the first read gave | `checkIntake` | The file on disk | A later read | After one `register` |
|---|---|---|---|---|---|
| `75ec37c` | 4 sources | `src-odysseus` **ALLOWED** (License cell, 166 chars: `unverified (read as AGPL-3.0 …)`) | invalid: `sources.1.license: Too big: expected string to have <=120 characters` | `[]`, with a degradation | the file holds `["src-source"]` only |
| `715db6b` | 4 sources | `src-hermes-agent` (176 chars) **and** `src-odysseus` ALLOWED | invalid at `sources.0.license` and `sources.1.license` | `[]` | `["src-source"]` only |
| `c0e1534` | 4 sources | refused for all three `unverified` rows | valid | the same 4 | 5 sources |

The three defects:

1. **`seed()` wrote and returned an unvalidated watchlist.** It built
   `{ schemaVersion, sources, retired: [] }` from `parseWatchlistMarkdown` and never
   ran the schema over it. The first read handed out an entry the schema refuses, and
   every later read found the file invalid and read the Stoa as empty. The reader's
   return type, `WatchlistEntry[]`, promised more than anything had checked: a type
   cannot say "at most 120 characters".
2. **`checkIntake` failed open.** It refused only a license equal to `unverified`
   (after trimming and lowercasing), so every other string read as a verified
   license. A cell that wrote its reading next to the value, `unverified (read as
   AGPL-3.0 …)`, permitted intake from the very AGPL row DD-M9-6 had refused
   "whatever verification finds". FR-13.5 and SDD §4.7 allow two kinds of value: a
   verified license, or exactly `unverified`.
3. **`register()` wrote over a watchlist it could not read.** Writers took
   `watchlist()`'s empty answer for an invalid file as the truth. `register()` wrote
   a one-entry file over it, so the four seeded sources were gone, although
   `watchlist()`'s own comment says the file is never overwritten. `retire()` never
   wrote, because it found nothing to retire. It answered "no source … on the
   watchlist", which was false. This third defect was found while reproducing the
   first two; the Architect chose to fix it here.

A fourth, smaller one went with them. A row with a source id and no http(s) url (or no
tags) was skipped without a word (invariant §7). The Architect approved reporting it,
as part of the plan.

**Why the table could do this.** No test read the shipped table, and the seed tests
used a two-row fixture whose cells were all tidy. The one test that asserted "the seed
cannot write an invalid file" (`test/shared/stoa.test.ts`) ran over that tidy fixture,
so it checked the fixture and never the seed.

## 2. What changed

| File | Change |
|---|---|
| `src/shared/stoa.ts` | Adds `INTAKE_LICENSES` (ten MIT-compatible SPDX ids), `licenseStanding` (`permitted` / `unverified` / `incompatible`) and `issueReasons`. `checkIntake` refuses every license not on the list, and its refusal names the list. `parseWatchlistMarkdown` returns `{ entries, rejected }`: every source row is either a schema-accepted entry or a rejected row with its reasons. |
| `src/main/stoa.ts` | `seed()` validates the whole file with `watchlistSchema`, writes only what passed and returns that same value, reports every refused row in ONE degradation with how to recover, and logs them as `rejected` on the `seeded` event. A private `readWatchlist()` lets `register`/`retire` refuse while the file is invalid (`unreadable()`), leaving its bytes as they were. The researcher's licence note is chosen by standing. |
| `prompts/stoa/intake-incompatible.md` | New: the researcher's note for a license that is neither `unverified` nor on the list. It never says "nobody has checked". |
| `test/fixtures/stoa-seed-defects/WATCHLIST.md` | New fixture: a good row, an honest `unverified`, the real 166-character cell from `75ec37c`, `unverified (read as MIT)`, `Unverified — pending`, `AGPL-3.0`, and a row with no url. |
| `test/main/stoa.test.ts` | Ten new or changed cases through the real seed path (§6), a curation-refusal case that compares bytes, and the method-surface list gains `readWatchlist` and `unreadable`. |
| `test/shared/stoa.test.ts` | Intake tables (each listed id permits, case and space ignored; seven shapes refuse; the list itself pinned). The reader's tests move to the new return shape, with cases for an over-long cell, an ftp link and an empty Tags cell. |
| `test/mutation/stoa-seed-fail-closed.json` | The round's spec: 15 real mutants and one control (§6). |
| `docs/DECISIONS-LOG.md` | The Architect's three answers, the approved plan and the mechanical choices, recorded before the code. |
| `docs/sdd/SDD.md` | §4.7's `license` comment states the fail-closed rule and where the list lives. |
| `docs/IMPLEMENTATION.md` | R13's mitigation says intake fails closed, citing §4.7. |
| `docs/progress/M5b.md` | A post-close repair note at the end of M5b's archived record. No box changes, and the resume block in `docs/PROGRESS.md` is unchanged because the next step is unchanged. (Written first at the end of M5b's section in `docs/PROGRESS.md`; #73 moved that section into this file while the branch was open, and the merge put the note here.) |
| `docs/implementations/2026-10-11-stoa-seed-fail-closed.md` | This document. |

`docs/stoa/WATCHLIST.md` is not touched: the Architect chose "code only", and #72 already
holds both cells at exactly `unverified`.

## 3. Implementation approach

**Validate at the bridge, and again at the file.** `parseWatchlistMarkdown` is the one
place a human-maintained cell becomes a typed entry, so the schema runs there, row by
row. A line whose id cell is not a source id (the header, the separator, prose) is
skipped, as before. A line that IS a source row comes back in exactly one of two
lists, `entries` (what `watchlistEntrySchema.safeParse` returned) or `rejected`
(`{ id, reasons }`). `seed()` then runs `watchlistSchema` over the assembled file,
because one rule belongs to the file and not to any row: at most 200 sources. It
writes `whole.data`, or `EMPTY_WATCHLIST` if the file-level check failed, and returns
the same value. The written bytes and the returned value come from one object, so the
first read and every later read agree by construction, not by coincidence.

**One degradation, not one per row.** `DegradationLog.report` keys an entry by its
cause and *refreshes the detail* on a repeat (`src/main/degradations.ts`), and the Stoa
reports every degradation under the single cause `odeon/stoa` (`src/main/index.ts`).
One report per refused row would have left only the last row visible. So the seed
gathers its faults into one sentence. That sentence names each row and its reasons,
says the seed runs once, and says how to make it run again. The `seeded` event carries
the same list as `rejected`, and a file-level refusal as `refused`, because a
degradation can be overwritten by a later one and `log.jsonl` cannot.

**Intake: one pure function, three answers.** `licenseStanding(license)` trims,
compares ignoring case, and returns `unverified` for exactly that word, `permitted` for
a listed id, and `incompatible` for anything else. `checkIntake` reads only it, as it
read only the license before (DECISIONS-LOG 2026-08-28, M5b.2). The researcher's note
comes from the same function through `INTAKE_NOTES`, so the plan's `intakePermitted`
and the note cannot disagree: both derive from `entry.license`.

**Writers ask a question readers do not.** `watchlist()` still answers an invalid file
with an empty list and a degradation, which is right for a reader (M5b.1: a malformed
watchlist reads as empty and is never rewritten). `register` and `retire` now ask
`readWatchlist()`, which returns `ok: false` with the file and its reasons, and they
refuse through `refuse()` so that each refusal is logged as `register-refused` /
`retire-refused` (NFR-13). The order of the guards is unchanged: a caller who is not
the Architect is refused before the file is read (R1), and a malformed draft is refused
before the file is read.

**Recovery is a sentence, not a feature.** The seed runs only when `watchlist.json` is
absent, so a row fixed in the table never comes back on its own. Every message on this
path (the seed's, `watchlist()`'s and the writers') says the same thing: repair the
file, or move it aside and restart. Moving it aside seeds again from `docs/stoa/`, and
anything registered since the seed is still in the moved file.

## 4. Mathematical / statistical details

There is no formula here. These are the counts the claims rest on:

- **Lengths.** The schema caps `license` at 120 UTF-16 code units (zod's string length,
  `z.string().max(120)`). The cell on `main` at `75ec37c` was 166; at `715db6b`
  Hermes Agent's was 176; both are rejected. The fixture's `unverified (read as MIT)`
  is 24, short enough that intake, not length, has to refuse it, and the test file
  asserts the 166 (`toHaveLength(166)`).
- **The allowlist.** Ten identifiers, matched as `id.toLowerCase() === value` where
  `value = license.trim().toLowerCase()`. Nothing else is normalised, so `MIT (read
  2026-10-03)` is not `MIT`, and `MIT?` is not either. Both are tested.
- **The file-level rule.** `sources` holds at most 200. The case that proves the seed
  checks it before writing uses 201 rows; with the check removed, the seed writes 201
  and every later read refuses the file.
- **The round.** 16 mutants, one of them the control. "15 of 15 killed" counts the real
  mutants; the control is not a kill.

## 5. Design decisions

Asked of the Architect through the questions protocol, and recorded before any code
(DECISIONS-LOG 2026-10-11):

- **Allowlist of MIT-compatible ids, rather than any SPDX id or a prefix rule.** A
  prefix rule (refuse values beginning `unverified`) would still permit `TBD`,
  `pending`, `unknown` and `AGPL-3.0?`. It widens a denylist; it does not close one. A
  list of every SPDX id would let a verified AGPL row permit intake, contradicting
  DD-M9-6. Being on the list is necessary, never sufficient: FR-13.5's attribution and
  decision memo still apply.
- **Code only.** No table edit here (#72 already fixed both cells, and editing them
  here would have conflicted with it), and no CI test over the shipped table. That test
  was offered with a Gymnasium row (GYM-012) to approve or hold, and declined. A bad
  cell is caught the first time a home seeds from it, as a visible degradation, rather
  than in CI.
- **The overwrite is fixed here**, not spun off.
- **Report a source row with no url or tags.** Approved as part of the plan.

Mechanical choices, inside those answers:

- **The schema is not narrowed.** Making `license` an enum of `unverified` plus the
  list would refuse every existing `watchlist.json` whose license someone typed freely.
  An enum rename is a schema migration, and so is narrowing a free-text field. The
  field stays a string of 1 to 120 characters; what it permits is a separate question.
- **A third researcher note, not a reworded second one.** `intake-refused.md` says
  "nobody has checked", which is false of a license somebody read. Rewording it to fit
  both cases would weaken the honest case to cover the other, so the other gets its
  own file (invariant §8: the words live in `prompts/stoa/`).
- **A refused row is left out, not repaired.** Splitting `unverified (…)` into
  `unverified` plus a note would have kept the row. It would also have written a
  second convention into the bridge, one the table's own header (as amended by #72)
  rules out: the License cell holds a value and nothing else.
- **Only an http(s) url is taken from the Source cell.** Handing the raw cell to
  `.url()` would admit `ftp:` or any other scheme a cell happened to hold. There is a
  test for it.
- **Not changed:** two rows sharing an id are not refused (see Follow-ups); a malformed
  watchlist still reads as empty and is never rewritten.

## 6. Verification

Run from the repository root.

1. **The tests through the real seed path:**
   ```bash
   npx vitest run test/shared/stoa.test.ts test/main/stoa.test.ts
   ```
   On win32 (Node v20.16.0) at `fbb72ce`: 2 files, 108 passed. The cases that carry the
   claims are in `test/main/stoa.test.ts`, under "a table row the schema refuses":
   - the first read deep-equals what a second driver over the same home reads, and
     what `parseWatchlist` makes of the file, with no degradation on the later read;
   - one degradation names `src-fixture-long (license: … 120 …)` and
     `src-fixture-nourl (url: …)`;
   - the `seeded` event carries both rows as `rejected`;
   - `plan()` gives `intakePermitted` true for the MIT row only;
   - each researcher note's sentence is present for its own standing and absent from
     the others;
   - 201 rows seed nothing, say so, and agree on a later read.

   Under "a damaged watchlist": `register` and `retire` refuse over a seeded file whose
   one fault is an over-long cell, each naming the file and `sources.0.license`, and the
   file's bytes are unchanged.
2. **The new tests fail against the old code.** In a detached worktree at `fbb72ce`,
   with `src/shared/stoa.ts` and `src/main/stoa.ts` checked out from `c0e1534`, the
   result was:
   - `test/main/stoa.test.ts`: 38 tests, 28 passed, 10 failed. All eight cases of the
     new describe block failed, plus the curation-refusal case and the method-surface
     list, which names two methods the old class lacks.
   - `test/shared/stoa.test.ts`: did not load, because it imports `INTAKE_LICENSES`
     and `licenseStanding`, which the old module does not export.
3. **The mutation round** (TEST-STRATEGY §10), from a detached worktree outside
   OneDrive at `%TEMP%\eph-stoafix2`, `node_modules` linked, win32, Node v20.16.0:
   ```bash
   node scripts/mutate.cjs test/mutation/stoa-seed-fail-closed.json --check
   node scripts/mutate.cjs test/mutation/stoa-seed-fail-closed.json --retries 6 --retry-wait 45
   ```
   At `fbb72ce`: `baseline: PASS — 2 files, 108 passed`, then `ROUND OK — 15 of 15 real
   mutants killed; the control survived`. The first round, at `7c9b51a`, reported one
   survivor, `unverified-given-other-note`. That survivor was a missing test, not an
   equivalent mutant. The fixture's `src-fixture-unverified` Notes cell said "the note
   that says nobody has checked", and the study prompt renders a row's notes into its
   question. So the phrase the test asserted came from the fixture, and the test stayed
   green with the two notes swapped. The cell was reworded, a guard now asserts that no
   seeded fixture row contains any note's sentence, and each note's sentence is
   asserted absent where it does not belong (`fbb72ce`). In the first round the
   control's first run produced no report, which is the shape the free-memory gate
   leaves (free memory measured 0.92 GB, then 1.02 GB, in the minutes before, against
   `MIN_FREE_BYTES` of 1 GB). The round retried after 45 s, as it is built to.
4. **The Definition-of-Done gate**, run in a detached worktree outside OneDrive at
   `%TEMP%\eph-stoagate` holding `fbb72ce` (tree `310d72c`, checked with `git
   write-tree`), win32, Node v20.16.0, `node_modules` linked to the main checkout's
   (lockfile matched: 378 entries, no mismatch, the 50 absent all optional):

   | Run | Free at start | What ran | Result |
   |---|---|---|---|
   | 1 | 2.97 GB | `npm run typecheck`; `npm run lint`; `node scripts/check-invariants.cjs`; `npm test` | typecheck exit 0 (28 s); lint exit 0 (19 s); invariants ok, reachability 189/199; suite 240 files, **5042 passed, 1 failed**, 8 skipped (299 s) |
   | 2 | 3.86 GB | `npx vitest run --reporter=json` | the same one failure, and nothing else |
   | 3 | 2.95 GB | `npm run test:coverage`; `node scripts/check-coverage.cjs` (CI's two steps) | **240 files passed; 5043 passed, 8 skipped** (174 s); "coverage floors ok (17 subsystems on win32; 19 untested modules, all recorded)" |
   | 4 | 2.55 GB | all of the above, plus `check-readme-current` and `check-attribution`, on the merge of `main` at `8a3a3a7` (#73) into this branch: commit `6f28c4b`, tree `3370b9c` | typecheck exit 0 (28 s); lint exit 0 (34 s); invariants ok; **241 files passed; 5055 passed, 8 skipped** (168 s); floors ok; README current for M8; attribution ok |

   **The failure in runs 1 and 2** was `test/scripts/check-invariants.test.ts` › "the
   CLI, run as CI runs it › exits 0 over this repository". Its `spawnSync` child was
   killed at the test's `CHILD_TIMEOUT_MS` (20,000 ms; the test took 20,122 ms), under
   the CPU load of the full suite while another full suite ran in a second worktree on
   this machine (free memory fell from 2.97 GB to about 0.5 GB during run 1). Four
   checks put it on load, not on this change:
   - the checker alone exits 0 in 3 s on this tree;
   - that test file alone passes 20 of 20 (5.5 s);
   - run 3 passed it;
   - GYM-008's gate hit the same test the same way on 2026-10-03, before this change
     existed, and recorded it as a follow-up.

   Not run: `main` under the same load, for comparison.

   **Coverage:** the `stoa` row rose on every metric (lines 96.48 → 97.53, branches
   84.77 → 88.89, functions 98.31 → 98.48, statements 95.5 → 96.88). Every rise is
   inside the 5-point `ratchetLag`, so the record is not stale. It was not ratcheted:
   a raise needs three corroborating runs per platform, and the linux ones exist only
   in CI. The docs commit adds no code; its checks are in its commit message.

**The upgrade path, for a home seeded between 2026-10-02 and 2026-10-11.** The
over-long cell was on `main` from `9723c10` (#66, 2026-10-02) to `c0e1534` (#72,
2026-10-11). A home whose Stoa first seeded from a build in that window holds a
`watchlist.json` that does not parse. After this change, that home reads the Stoa as
empty, shows a degradation naming the file and reason, and refuses to register or
retire. It does not repair the file, by design. To recover, move
`<home>/agora/stoa/watchlist.json` aside and restart: the seed runs again from the
fixed table and gives four sources. A home where a source was registered *before* this
change, over the invalid file, holds a valid one-entry file, which no check can tell
from a deliberate one. If the Stoa lists only hand-registered sources and none of the
four seeded ones, move the file aside the same way and register those sources again;
they are in the moved file.

**Not proven.**
- Everything above ran on win32 only, not on CI's ubuntu.
- The app was not launched: the degradation was observed in the driver's callback,
  not in the window's degradation panel.
- The panel's `intakeBlocked` text for the new refusal was not seen rendered.
- No existing home on this machine was inspected for the 2026-10-02 to 2026-10-11
  state.

**Follow-ups (not in scope).**
- Two table rows sharing an id are both seeded. `plan()` finds the first, and
  `retire()` removes both and keeps one in `retired`, so the second is lost.
- The Stoa reports every degradation under the one cause `odeon/stoa`, so a later
  Stoa degradation in the same session replaces the seed's text in the panel. The log
  keeps both.

## 7. Related docs

- [DECISIONS-LOG](../DECISIONS-LOG.md): 2026-08-28 (M5b.2, unverified permits study and
  refuses intake), 2026-10-02 (DD-M9-6, the AGPL row), 2026-10-11 (the #72 review entry
  that named this task, and this change's entry)
- [SDD §4.7](../sdd/SDD.md): the watchlist entry and its `license` rule
- [SRS FR-13.5, FR-13.7](../srs/SRS.md) and
  [ADR-0017](../adr/ADR-0017-stoa-research-department.md) R3
- [TEST-STRATEGY §10](../TEST-STRATEGY.md): the mutation round
- [The M5b build record](2026-08-28-m5b-stoa-and-modes.md): where the seed and the
  intake split were built, and [M5b's progress record](../progress/M5b.md), which
  carries this repair's note
- [`docs/stoa/WATCHLIST.md`](../stoa/WATCHLIST.md): the table the seed reads

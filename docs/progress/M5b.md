# Ephesus — Build Progress: M5b (archived)

Moved verbatim from [`../PROGRESS.md`](../PROGRESS.md) on 2026-10-11. Nothing below
was rewritten except relative links, rebased by one folder. The live register — the
*Resume here* block, the milestone index and every open milestone — is in
`PROGRESS.md`; an item this milestone left open is carried there.

---

## M5b — The Stoa + company modes (plan drafted 2026-08-28 at M5 close)

Derived per BUILD-PROMPT §5 from IMPLEMENTATION M5b + ADR-0017/0018 + SRS
FR-13/FR-14 + SDD §4.7/§7.7/§9 + TEST-STRATEGY S-STOA/S-MODE/E-STOA. Execute
in order; every package tests against the fake engine per-PR. The Architect's
2026-08-28 decision: the milestone also carries the floor-art intake (M5b.5) —
the LimeZu purchase (Modern Interiors + Modern Office Revamped) is made.

- [x] **M5b.1 Watchlist + `stoa.ts` core** — `agora/stoa/` layout (SDD §2);
      watchlist schema §4.7 (schemaVersion 1, strict, validators in
      `src/shared/`); accessors with **Architect-only mutation enforced in the
      handler** (the `gym.verdict` pattern — FR-13.1); seeding from the repo's
      `docs/stoa/` at first run (FR-13.7, the gymnasium-seed pattern —
      *including* `briefs/`, per the audit's finding-3 lesson); `stoa:` IPC
      group (`watchlist() register(entry) retire(id) briefs() brief(id)`);
      the Stoa panel v1 — the reading desk: paste a URL + tags, see the list.
      *Docs: ADR-0017, FR-13.1/13.7, SDD §4.7. Tests: validators table-driven;
      seeding; non-architect register/retire refused at the handler; UI stays
      a projection. Risk: the id/pin/license fields are the provenance chain —
      no field invented beyond §4.7.*
      *Evidence: 70/70 new cases green (`test/shared/stoa.test.ts` 43 —
      table-driven §4.7 schema incl. strict-reject of a smuggled
      `registeredBy`, seed-table reader, R1 refusals, studiable/intake gates;
      `test/main/stoa.test.ts` 27 — seeding, register/retire, damaged file,
      read-only archive surface). LIVE Electron run against a temp
      `EPH_HOME`: the STOA tab seeded 3 sources from `docs/stoa/WATCHLIST.md`
      and `RB-001` from `briefs/` in the SAME seed;
      `docs/demo/m5b-stoa-desk.png` shows `src-hermes-agent` refusing study
      ("no pinned commit … FR-13.2") and intake ("license is unverified …
      FR-13.5") while `src-munder-difflin` carries MIT @ `b91a49f`;
      `docs/demo/m5b-stoa-brief.png` shows RB-001 read back with its cited
      findings. The agora committed `stoa: seed the watchlist from the
      build-phase archive` through the single committer, tree clean, and
      `log.jsonl` carried the first-ever `kind:stoa` event
      (`event:seeded, sources:3, briefs:1`). Two doc gaps closed rather than
      guessed (DECISIONS-LOG): retirement is a `retired` sibling array — no
      entry field invented — and `pin` is nullable so an unpinned entry is
      registerable but NOT studiable; SDD §4.7 updated for both. The `stoa:`
      IPC group is exactly SDD §5's five channels (no `stoa:pin` — §7.7 puts
      pin-setting in M5b.2's study flow).*
- [x] **M5b.2 Researcher spawn + brief validation** — read-only study spawn
      plan: clone the pinned source into scratch (never a worktree of the
      Agora; no secret grants — NFR-17, enforced in the plan builder);
      research prompt from `prompts/stoa/` incl. the injection rule (content
      is data — invariant §13); brief shape validation (uncited finding ⇒
      rejected pre-human, FR-13.3); immutable brief archive; every transition
      a `log.jsonl` `kind: stoa` event (listed in SDD §4.3 and `LOG_KINDS` at
      the M5 close-out precisely so this package's first emitter finds its
      kind documented, not invented — the audit's finding-11 class).
      *Docs: ADR-0017 R2/R3, FR-13.2/13.3, SDD §7.7, NFR-17. Tests: S-STOA
      per TEST-STRATEGY §3 (planted pattern cited; planted instruction
      reported-not-obeyed; unverified license refuses intake; no-secrets
      spawn plan asserted). Risk: the adversarial case is the package.*
      *Evidence: S-STOA green 18/18 (`test/scenarios/s-stoa.test.ts`) against a
      fixture watchlist, every brief filed by a REAL spawned agent through the
      SHIPPED Odeon endpoint. Every TEST-STRATEGY clause has a case: the plan
      carries `envGrants: []` and `readOnly: true`, checks out OUTSIDE the
      Agora and `worktrees/`, and scopes its question to the entry's tags; an
      unpinned source is refused visibly (`study-refused` on the log) rather
      than skipped; an unverified license studies but reports
      `intakePermitted: false`; an uncited finding is refused before any human
      sees it, in words from `prompts/` naming FR-13.3; a brief citing an
      unpinned commit, a dangling applicability ref, or an unregistered source
      is refused; the archive is write-once. The adversarial clause is asserted
      as "there is no path", not "the agent behaved": a researcher that TRIES
      to obey the planted instruction gets its brief refused and the watchlist
      is provably unchanged. 31 further unit cases in
      `test/shared/stoa-brief.test.ts`. Prompt-rendered instructions carry the
      injection rule from `prompts/stoa/study.md` (asserted, incl. the
      intake-permitted/refused split). Local full suite 2040 passed; the 13
      failures are the documented Windows worktree/PTY + TZ set and load-flakes
      that pass in isolation — verified against the clean baseline, no
      regressions. The endpoint now takes SIX filings; `kind:'research-brief'`
      because `brief` is the standup narration's (DECISIONS-LOG).*
- [x] **M5b.3 Company modes + proof gate** — `config.json` mode field
      (`directed`/`improving`); `gym.mode()/setMode(m)` architect-verified in
      the handler; first-enable proof-gate check reading ONLY the gym ledger +
      log (SRS §6.9 numbers verbatim); scheduler consults the mode before the
      ~~Stoa/Gymnasium cadences~~ Stoa cadence *(corrected at the close-out
      audit: only a Stoa cadence trigger exists — the gym cadence is M7's per
      IMPLEMENTATION)* fires; mode tag on autonomous records (FR-14.1);
      status-strip mode chip; breaker rung-3 auto-revert (FR-14.5).
      *Docs: ADR-0018 (normative), FR-14, SDD §9. Tests: S-MODE per
      TEST-STRATEGY §3 — premature enable refused listing missing evidence;
      fixture ledger meeting §6.9 enables; no agent-side path can set the
      mode; auto-revert lands on the ledger. Risk: the gate reads the book of
      record, never a computed cache (invariant §11's spirit).*
      *Evidence: S-MODE green 13/13 (`test/scenarios/s-mode.test.ts`), with the
      passing ledger BUILT by driving the shipped endpoint and verdict path —
      three proposals through proposed → verdict → landed → measured, two
      validated, one citing `RB-001` — so the gate reads a ledger the company
      actually produced. Every clause covered: an empty ledger refuses listing
      what is missing; two-through-the-loop still refuses; four non-architect
      actors are refused on a PASSING ledger; no filing at the Odeon reaches the
      mode at all; the Stoa cadence does not fire in `directed` and its record
      carries `mode: improving` when it does (FR-14.1); a rung-3 revert lands on
      the ledger document attributed to `breaker` and preserves `everEnabled` so
      restoring is not a re-proof. 45 unit cases (`test/shared/mode.test.ts` 32,
      `test/main/modes.test.ts` 13) + 2 scheduler-gate cases + 3 ledger
      regressions. **Latent defect found and fixed:** `Gymnasium.append()`
      deleted everything below its table, so FR-14.5's mode section would have
      vanished at the next proposal — the M5 close-out finding-1 class one row
      further down (DECISIONS-LOG). Local suite 2104 passed; the 12 failures are
      the documented Windows worktree/PTY + TZ set and load-flakes.*
- [x] **M5b.4 Brief → proposal flow + E-STOA** — Artemis's ranking playbook
      (`prompts/`), proposals citing brief ids in evidence refs (FR-13.4), the
      standup brief's gym-slice section folds the Stoa in (FR-13.6); E-STOA
      eval per TEST-STRATEGY §6 over the fixture source.
      *Docs: ADR-0017, FR-13.4/13.6, UC-14 step 5. Tests: proposal-citing
      shape; brief-fact refs resolve; E-STOA fixture run recorded. Risk:
      Artemis ranks — the harness never files a proposal itself (ADR-0005).*
      *Evidence: the chain is real end-to-end — a brief archived through the
      shipped endpoint, a proposal citing it accepted, and a proposal citing
      `RB-404` REFUSED with the id named (S-STOA now 20/20). The link is the
      citation itself: `citedBriefIds` reads the evidence refs, the Gymnasium
      refuses ids not in the archive, and the `proposed` log event carries them
      so the proof gate counts Stoa-seeded proposals from the log alone.
      Artemis's ranking playbook is `prompts/stoa/rank.md` (rank, never decide;
      cite the brief; keep the internal evidence; file fewer than you found).
      FR-13.6: the standup's gym-slice section now reports sources watched and
      briefs archived, plus the company mode (FR-14.1) — omitted rather than
      zeroed when there is no Stoa (3 cases in `test/shared/brief.test.ts`).
      E-STOA: `test/fixtures/stoa-source/` carries two planted patterns, noise
      (a roadmap), and a planted injection; `test/evals/e-stoa.ts` scores a
      brief against them and `e-stoa.test.ts` (15/15) asserts the RUBRIC
      discriminates — a good brief passes, and uncited / noise-only / obeyed /
      silently-ignored each fail it. Local suite 2124 passed; the 12 failures
      are the documented Windows worktree/PTY + TZ set and load-flakes (the one
      unfamiliar name passed in isolation).*
      *Owed to a real-engine session (recorded, never faked): E-STOA's
      LLM-judged half — "is this applicability mapping honest?" and "does the
      prose match the file it cites?" — needs a rubric-scoring model over a
      real researcher run. The deterministic half runs in CI today.*
- [x] **M5b.5 Floor art intake (the purchased packs)** — drop the LimeZu
      **Modern Interiors v41.4** and **Modern Office Revamped** 16×16 sheets
      into the gitignored tileset drop; author their `*.tiles.json` layout
      maps (validated by `src/shared/tileset.ts`); retire the interim Kenney
      staging; `ATTRIBUTION.md` rows (licence terms + the required credit
      link) and the status-strip credit; §7 bar re-verified: 2× integer scale,
      pixel-snap, ≤8 colors per screen via the palette pass; floor screenshot
      to `docs/demo/`.
      *Docs: UI-DESIGN §7, ATTRIBUTION.md rules. Tests: token/contrast checks
      stay green; tile-map validator on both packs; scene-state assertions
      unchanged (art is presentation). Risk: the sheets NEVER enter the repo
      (licence forbids redistribution of the asset itself); characters stay
      *Evidence: the floor paints from the purchased pack —
      `docs/demo/m5b-floor-limezu.png` is a live Electron capture showing
      LimeZu walls, a cream/marble floor, tan paths and an indigo temple block,
      with the required credit on the status strip: `tileset: LimeZu Modern
      Interiors (Room Builder 16x16) — LimeZu — limezu.itch.io`. Two sheets in
      the gitignored drop (`Room_Builder_16x16` and the office
      `Room_Builder_Office_16x16`); the furniture sheets were extracted, found
      to be unmapped, and DELETED rather than left for `import.meta.glob` to
      bundle. Frame indices were measured, not guessed: candidates scored by
      colour variance + opacity, then each rendered back from its own index and
      tiled 3x3 to prove both the arithmetic and the seam (DECISIONS-LOG). The
      first mapping paired cream with mint and read as a chessboard; re-mapped
      to warm near-neighbours with indigo reserved for the temple, which is
      §1's ≤8-colour bar honoured rather than merely claimed. Kenney staging
      retired; ATTRIBUTION carries both LimeZu rows with the real licence terms
      (use yes, redistribute no, credit required), an exact restore path, and
      the do-not-use list (free tier, RPG-Maker build, and the character
      generator that is never run — rule 3). `*.tiles.json` is now committed
      while the sheets stay ignored, so the restore path is reproducible.
      `test/renderer/tileset.test.ts` ~~36/36~~ 33-or-35 depending on whether
      the local drop holds the sheets *(corrected at the close-out audit — the
      exit review's own drop-guard numbers, re-reproduced there)*, including a
      new block that
      validates the REAL drop when present — frame bounds, columns, integer
      scale, sheet existence — and is inert in CI where the drop is empty.
      Local suite 2136 passed with exactly the 9 documented Windows
      worktree/PTY + TZ failures and no flakes.*
      *Note for the build-state block: the interiors download is named by its
      itch slug (`moderninteriors-win.zip`), not "v41.4" as BUILD-PROMPT's note
      assumed — recorded in ATTRIBUTION's restore path.*
      procedural (rule 3).*
- [x] **M5b.6 Scenario suites + exit review** — S-STOA and S-MODE green in CI;
      E-STOA recorded; then the exit demo: **one real research cycle through
      the app** — a URL registered on the reading desk, a study producing an
      archived provenance-valid brief, and a GYM proposal citing it in the
      Architect's queue (IMPLEMENTATION M5b exit; SRS §6.8 as S-STOA, §6.9 as
      S-MODE). The proof gate itself is *met later by operation* — this
      milestone builds the machinery that will measure it.
      *Evidence: S-STOA green in CI (run 33186729216) and S-MODE green in CI
      (run 33188310155), both on ubuntu where the Windows worktree/PTY failures
      do not apply; E-STOA recorded (15/15, deterministic half). **The exit
      demo ran through the app**, end to end, on a fresh home:
      `docs/demo/m5b-cycle-1-registered.png` — a URL typed into the real
      reading desk and registered by clicking REGISTER (real inputs, real IPC,
      nothing written by the harness): `src-opencode-sdk-js`, MIT, pinned
      `9f3c1de`. `docs/demo/m5b-cycle-2-brief.png` — the study's brief archived
      as **RB-002**, with `log.jsonl` recording
      `brief-archived {findings: 2, directivesReported: 1}`: both findings
      cited, and the source's instruction to its reader REPORTED rather than
      obeyed. The artifact itself is exported to
      `docs/demo/m5b-cycle-brief-RB-002.md`.
      `docs/demo/m5b-cycle-3-proposal.png` — **GYM-006** waiting in the
      Architect's queue, with `gym proposed {briefs: ['RB-002']}` on the log:
      the citation link, machine-checkable. The brief and the proposal arrived
      as files in a real agent outbox, so Hermes, the Odeon endpoint, the Stoa
      archive and the Gymnasium each ran their SHIPPED path; only the
      researcher's engine is stood in for. The router refused three malformed
      attempts on the way (bad id, missing `needs_human`, `requires_reply:
      false` on a `propose` — ADR-0003's obligation table), each parked in
      `outbox/.rejected/` with its reason.*
      *Two defects found BY the demo and fixed (DECISIONS-LOG): the reading
      desk had no pin field, so every source registered from it was
      permanently unstudiable — M5b.1 deferred pin-setting to M5b.2 and M5b.2
      built a `plan()` that only reads it; and the proof gate read every
      SEEDED ledger row as a gating violation, which — a violation being
      absorbing — made `improving` unopenable on any company that inherited a
      build-phase archive. Every unit test had passed because each synthesised
      the log it wanted; only the running app read the real seeded ledger
      beside a real fresh log.*
- [x] **M5b exit review** — S-STOA + S-MODE green in CI; the real research
      cycle evidence; PROGRESS + docs synced.

### M5b exit review (2026-08-28) — verdict: DONE

Every exit criterion verified **by execution**, not by inspection.

| Criterion | How it was proved |
|---|---|
| S-STOA green in CI | run 33186729216 (ubuntu), 18 cases; 20 after M5b.4 added the brief-citing pair |
| S-MODE green in CI | run 33188310155 (ubuntu), 13 cases |
| E-STOA recorded | 15/15 — the rubric's own discrimination test, over `test/fixtures/stoa-source/` |
| One real research cycle through the app | ~~the three `docs/demo/m5b-cycle-*.png` captures + the archived `RB-002` + `GYM-006` on the ledger citing it~~ **AMENDED at the close-out audit:** the original demo was machinery-real but **source-fake beyond its disclosed stand-in** — pin `9f3c1de` does not exist in the repository the watchlist named, and the GYM-006↔RB-002 pairing survived in no artifact ("on the ledger" was inaccurate: it was in the queue of an ephemeral home). The row is satisfied by the close-out re-run against a REAL pin instead: [`m5b-cycle-real-source.txt`](../demo/m5b-cycle-real-source.txt) — munder-difflin @ `b91a49f`, verified remotely (`gh api`) and locally in-run, register → plan → brief → citing proposal through the shipped path, the pairing in the log |
| PROGRESS + docs synced | this file, DECISIONS-LOG, SDD §2/§4.7/§9, ATTRIBUTION, and `docs/implementations/2026-08-28-m5b-stoa-and-modes.md` |
| The INTEGRATED stack green in CI | run **33192290049** on `feature/m5b-6-suites-exit` — all six packages plus the drop-guard fix, on ubuntu |

**Four defects found by running the thing, all fixed in-milestone:**

1. `Gymnasium.append()` deleted everything below its table (M5b.3) — FR-14.5's
   mode section would have vanished at the next proposal.
2. The reading desk had no pin field (M5b.6) — every source registered from it
   was permanently unstudiable. M5b.1 deferred pin-setting to M5b.2; M5b.2
   built a `plan()` that only reads it. Neither package owned it.
3. The proof gate read every SEEDED ledger row as a gating violation (M5b.6) —
   and a violation is absorbing, so `improving` could never be enabled on any
   company that inherited a build-phase archive. Every unit test passed because
   each synthesised the log it wanted.

4. The drop-validation block guarded every check on "a map exists" (M5b.6) —
   committing the maps meant CI had maps and, correctly, no sheets, so the two
   checks that open a PNG failed on ubuntu while passing locally. Found by CI
   on the push, fixed, and verified both ways: 33 cases with the sheets moved
   aside, 35 with them present.

The first three are the seam-blindness class the M3, M4 and M5 audits each found a
different way: correct halves that only disagree when the running system puts
them side by side. The M5b answer was the same one that worked before — run the
demo, and believe the demo over the suite.

**Carried, unchanged:** the 2026-09-11 metric sweep (GYM-002/003/004 ledger
checks + GYM-003's live-quit evidence run) did **not** fall inside this run —
today is 2026-08-28 — so it is carried to the next milestone window rather than
booked here. GYM-001's own check is due 2026-09-25.

**Owed to a real-engine session (recorded, never faked):** E-STOA's LLM-judged
half; a study whose checkout and pin are recorded by the researcher rather than
typed by the Architect (the "pin before study" path, which still has no
implementation — the desk's pin field is the Architect's route, not the study's).


**Standing due-dates to carry:** the 2026-09-11 metric sweep (GYM-002/003/004
ledger checks + GYM-003's live-quit evidence run) falls inside this
milestone's window — book it in the exit review if unmeasured by then.

### M5b close-out audit (2026-08-29) — verdict: DONE, with the record amended and audit fixes landed

Independent two-agent audit at milestone close, the M0–M5 pattern:

- **spec-verifier** (verification by execution): the machinery half VERIFIED —
  gate green (attribution over 121 commits) · S-STOA 20 / S-MODE 13 / E-STOA 15
  exactly as recorded · 194 M5b cases green together · all full-suite failures
  classified into the recorded environmental set, every extra green in
  isolation · three of four in-milestone defect regressions named and passing ·
  intake clean (sheets ignored, maps committed, ATTRIBUTION complete) · CI
  green on `ed46cad`. **But the "one real research cycle" exit row was
  NOT verified as written:** the demo's pin `9f3c1de` exists in no repository
  the watchlist named, the cited path is another repo's layout, and the
  GYM-006↔RB-002 pairing survived in no artifact. The disclosed stand-in
  covered the engine, not the source.
- **doc-guardian** (design conformance): **no invariant violations** — R1/R2/R3
  enforced mechanically, §6.9 verbatim and pinned, atomic/append-only/tokens
  clean, art intake fully conformant, characters procedural — plus five
  deviations and a tail of contained/nit findings, listed below.

**The record is amended, and the row re-proven** (evidence integrity is the
audit's first duty to itself): the exit-review row now says what the original
demo was, and the cycle was re-run at close against a REAL source — munder-
difflin @ `b91a49f`, verified on GitHub and against a local clone inside the
run — through the same shipped path, captured to
[`docs/demo/m5b-cycle-real-source.txt`](../demo/m5b-cycle-real-source.txt).

**Findings FIXED at close (regression tests named; gate after fixes: typecheck
PASS · lint PASS · invariants PASS · touched suites 113/113):**

1. **`Stoa.brief(id)` skipped first-run seeding** — a fresh home whose first
   Stoa-touching action was a proposal citing a seeded brief false-refused it
   (the M5 audit's half-seeded-seam class, one method over). Seeds like
   `briefs()` now; first-call regression test.
2. **The cadence trigger's body was exercised by nothing** — S-MODE and the
   scheduler suite each rebuilt it inline (the rig's own copy-of-the-wiring
   defect class). Extracted to `stoa-cadence.ts`; the suites and `index.ts`
   run the same shipped tick, which also logs `sourceId`/`planned` asserted
   end-to-end.
3. **The FR-14.5 role heuristic was an untested substring test in `index.ts`**
   — `includes('improv')` would have reverted the mode over a stop on a
   mission hire named "process-improver-docs". Now `isImprovementRole` in
   `shared/mode.ts`: exact roles from ADR-0019's vocabulary, table-tested with
   the audit's own counter-example.
4. **`stoaWatchlist` leaked `registeredBy` past the view type** — field-picked
   projection now; the response is the `SourceView` and nothing more.
5. **SDD drift corrected:** §7.7's proof-gate location (modes.ts, not
   watch/gates.ts) + the cadence-heartbeat build-state note (an autonomous
   no-op must not read as work); §1.1 gains the `modes.ts` row;
   ATTRIBUTION's drop section no longer lists the committed maps as
   gitignored; M5b.3's "cadences" plural and M5b.5's "36/36" corrected in
   place above.

**Recorded, not fixed (DECISIONS-LOG, with reasons):** FR-14.1's mode tag
rides one record type until M7 gives it more autonomous acts to tag; the §6.9
`stoaSeeded` count reads log events only (a seeded archive under-counts — the
strict direction, waitable); Stoa spend attribution owed beside the gym-spend
deferral (M6.7); no path advances a pin yet ("Architect advances it" needs a
`stoa:pin` channel or the study-recorded pin — both on the owed list);
`prompts/stoa/rank.md`/`study-refused.md` are loaded by nothing until the
Artemis-ranking leg lands; the plan's `question` line is serialization-plus-
connective (invariant-§8-adjacent); the reading-desk pin fix has no renderer
regression test (no DOM harness exists — owed with M6.1's renderer work).

**Post-close repair (2026-10-11): the watchlist seed validates what it parsed, and
pattern intake fails closed.** Found while #72 was reviewed, and reproduced by running
the shipped code over `docs/stoa/WATCHLIST.md` as it stood on `main` at `75ec37c`:
`seed()` wrote and returned the table's rows unchecked, so a 166-character License cell
reading `unverified (read as AGPL-3.0 …)` was listed on a fresh home's first read with
intake ALLOWED (`checkIntake` refused only the exact string `unverified`), and every
later read found the file over the schema's cap and the Stoa empty. Behind it,
`register()` wrote a one-entry file over the watchlist it could not read. #72 fixed the
table's cells; this repair makes the code safe against the next one. Each source row
the schema refuses is now left out and named in one degradation, with how to seed it
again, and in the `seeded` log event; intake needs a license on an MIT-compatible
allowlist (SDD §4.7); `register` and `retire` refuse over an unreadable file and leave
its bytes alone. No box changes. Evidence: a mutation round with a certified control,
15 of 15 real mutants killed (`test/mutation/stoa-seed-fail-closed.json`).
[Record](../implementations/2026-10-11-stoa-seed-fail-closed.md); decisions in
DECISIONS-LOG 2026-10-11 (Stoa seed hardening).

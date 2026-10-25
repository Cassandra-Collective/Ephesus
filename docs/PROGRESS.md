# Ephesus — Build Progress

Tracks work-package completion per BUILD-PROMPT §5. Every tick carries a one-line
evidence note.

**A session resumes at [Resume here](#resume-here), not at the first unchecked
box.** That was this file's rule until 2026-10-11, and it had stopped working: the
first unchecked box was M6.9, deferred indefinitely, and the exits of M7, M8b and
M8c, which wait on the Architect, sat between it and the real next step. The block
below names the next step, what waits on the Architect, and what is deferred. A
session that ticks a box, closes a milestone or records a decision that changes the
next step updates the block **in the same commit**.

**Closed milestones are in [`progress/`](./progress/)**, one file per milestone,
moved there verbatim on 2026-10-11 with only their relative links rebased. A
milestone moves once every package in it is ticked or deferred by the Architect; an
item it leaves open is carried in the block below, with a link to where it is
recorded.

---

## Resume here

*As of 2026-10-25: R0.1 is merged as #102 and R0.2 is PR #104. R0.3 (`feature/r0-3-standards-learn-rust`), R1.0 (`feature/r1-0-home-design`), R1.1 (`feature/r1-1-eph-schema`), R1.2 (`feature/r1-2-eph-fs`), R1.3 (`feature/r1-3-eph-git`) and R1.4a (`feature/r1-4a-agora-store`) are stacked on it, pushed, and get their PRs as the one below each merges. A branch and a PR per package (DECISIONS-LOG, 2026-10-19).*

**Ephesus is being rewritten in Rust (ADR-0041, the Architect's direction of
2026-10-19), and the rewrite comes before everything below.** The Architect
approved [`RUST-PLAN.md`](./RUST-PLAN.md), crate Set A in
[`RUST-CRATES.md`](./RUST-CRATES.md), GYM-013's thresholds and the SRS wording on
2026-10-19. **R0 is DONE** (verdict below, 2026-10-19). **R1.0, the design, is approved**
(2026-10-20: `docs/sdd/SDD-RUST.md` §2 and §4). **R1.1 is done** (2026-10-20):
`eph-schema` and the per-crate coverage floors, seeded from CI. **R1.2 is done**
(2026-10-20): `eph-fs`, the one write path, the append-only log, and a home marked
`home.json` that leaves a TypeScript-era home untouched. **R1.3 is done** (2026-10-20):
`eph-git`, the single committer with its own OS lock, hermetic Agora git, and worktrees
that never delete work. **R1.4a is done** (2026-10-25): `eph-agora`'s one store for
records, the roster, the task ledger and its retention, the board, and the Agora's boot
order. On the way, the Linux run found an R1.3 defect, a killed `git init`'s lock that
refused every later boot, and the Architect had it fixed. **The next build step is
R1.4b, `eph-agora`'s book of record**: the log writer, its readers across rotation,
publish and subscribe, the degradation channel (which then carries `Booted::conditions`
and the committer's unwaited failures as `agora/commit` rows), the reader half of killed
mid-append, and the committer's detection of work an agent's own ignore file hides
(Architect, 2026-10-25). Its open choices are asked first (the Architect split R1.4 and
moved `DIAGNOSIS.md` to R7, 2026-10-20). Three constraints carry forward. A new crate's
first push is red at the coverage step until its floor is seeded from that push's CI
runs (ENGINEERING-STANDARDS §1.1). A `.tmp` file left by a crash before its rename is a
recorded residual until the boot that runs ADR-0034's probe (R7); meanwhile the Agora's
`.gitignore`, rendered at every boot, keeps `write_atomic`'s temporaries out of history
(SDD-RUST §2). And `tasks-archive.jsonl` has no reader yet: the phase that builds one
dedupes by id and lets the ledger win (SDD-RUST §4.5). A later
phase still starts only when he approves its package list, through AskUserQuestion.
Silence is not a yes. The TypeScript tree is frozen as
the behavioural reference and is deleted at R14 (parity). The rewrite is measured by
[GYM-013](./gymnasium/proposals/GYM-013-the-rust-rewrite-is-measured.md). The
packages are in *The Rust rewrite* section below.

**After parity, M9 resumes** in its approved order (BUILD-PROMPT §5; DD-M9-1,
DD-M9-18): M9 → M9b → M9d → M9e → M9f → M7b, which is the v1 boundary. M9c may run
beside M9b and M9d. M9g and M9h follow v1. M9's build is still not authorised:
whether it may start is asked at parity, through AskUserQuestion
([`M9-PLAN.md`](./M9-PLAN.md), [`PHASE-9-PLAN.md`](./PHASE-9-PLAN.md),
[`M9-GOAL.md`](./M9-GOAL.md)).

**Waiting on the Architect, not on a build session:**

| Open item | What closes it | Recorded in |
|---|---|---|
| **M7 exit**, **M8b exit**, **M8c exit** | One record: bench task T1's first live row, from M9.1. The three boxes stay open until it exists, and the Architect ticks them on it; a fifth manual run of `EXIT-M8.md` was rejected (DD-M9-7). M8's own exit row closes on the same run (M8.12). | [M7](./progress/M7.md), [M8](./progress/M8.md); M8b and M8c below |
| M7: re-run SRS §6.1's action half on a real repository | The Architect names the repository and consents to unattended agents holding `GH_TOKEN` grants against it | [M7](./progress/M7.md) |
| M7: E-PLAYBOOK's live drill | Owed with §6.1 (TEST-STRATEGY §6) | [M7](./progress/M7.md) |
| Gymnasium: GYM-007, GYM-009 and GYM-010 proposed and undecided; outcome measurements due | The Architect's decisions; the ledger's dates | [`gymnasium/LEDGER.md`](./gymnasium/LEDGER.md) |

**Deferred by the Architect:** M6.9, wiring the Herald into the application,
indefinitely since 2026-08-30. DD-M9-16 takes speech into M9e.3 as output only and
keeps every input path deferred. Recorded in [M6](./progress/M6.md).

## Milestones

| Milestone | State | Record |
|---|---|---|
| M0 — Skeleton | Done | [`progress/M0.md`](./progress/M0.md) |
| M1 — One real agent, both planes | Done | [`progress/M1.md`](./progress/M1.md) |
| M2 — The Agora + Hermes | Done | [`progress/M2.md`](./progress/M2.md) |
| M3 — Artemis + the Watch | Done | [`progress/M3.md`](./progress/M3.md) |
| M4 — The Library + engine breadth | Done | [`progress/M4.md`](./progress/M4.md) |
| M5 — The Odeon + Gymnasium v1 | Done | [`progress/M5.md`](./progress/M5.md) |
| M5b — The Stoa + company modes | Done | [`progress/M5b.md`](./progress/M5b.md) |
| M6 — The floor's face + the Herald | Done, on criteria the Architect amended 2026-08-30; M6.9 deferred | [`progress/M6.md`](./progress/M6.md) |
| M7 — The Harbor + the two outward missions | Packages done; exit open (above) | [`progress/M7.md`](./progress/M7.md) |
| M8 — The company you can leave running | Packages done; its exit row closes on the M8b/M8c run (above) | [`progress/M8.md`](./progress/M8.md) |
| M8b — The crew can act, and the briefing can be read | Packages done; exit open (above) | below |
| M8c — Bounded, and honest about itself | Packages done; exit open (above) | below |
| M9 — The harness is the product | Planned; build not authorised | below · [`M9-PLAN.md`](./M9-PLAN.md) |
| M9b–M9f — the agentic harness, to v1 | Planned; build not authorised | below · [`PHASE-9-PLAN.md`](./PHASE-9-PLAN.md) |
| M7b — The recursive company + shipping | Planned; follows M9f (DD-M9-18) | below |
| M9g–M9h — after v1 | Planned; follow M7b | below |
| **R0–R14 — The Rust rewrite** | Plan approved 2026-10-19; **R0 done**; R1.0 (the design) approved, and R1.1 (`eph-schema`, the coverage floors), R1.2 (`eph-fs`) and R1.3 (`eph-git`) done, 2026-10-20; R1.4a (`eph-agora`'s stores) done, 2026-10-25; R1.4b next. Runs before M9 | below · [`RUST-PLAN.md`](./RUST-PLAN.md) |

---

## M8b — The crew can act, and the briefing can be read (plan drafted 2026-09-09)

Derived from IMPLEMENTATION M8b + the numbered findings of
[the M8 exit run](demo/m8-onehour-aftershock.md). M8 proved the company survives
being left alone. That run then showed the other half of SRS §6.1 failing in a way
no unit suite could have predicted: **the crew detected everything and could act on
nothing.** Every package here closes a finding that run produced, which is why none
of them was planned before it.

- [x] **M8b.1 Install the profile bundle into the harness home** — Finding 8, the
      record's own *"single highest-value fix"*, and the direct cause of **both**
      failing action clauses of SRS §6.1. A profile bundle was read from the
      **repository**, so an instance activated in a real harness home had no
      runbooks to cite and no authority to act from.
      *Evidence: the adversarial pass is the entry here, not the suite. After 27
      green tests and 15 killed mutants the pass asked the question none of them
      encoded — can the agent actually OPEN the file the message names? It could
      not: `<home>/instances/<instance>/playbooks/` sits outside the spawn working
      directory with no grant at all, the same position `mailboxPermissions`
      already documents for the mailbox. A `Read`-only grant was added and minted
      at spawn — deliberately narrower than the mailbox's `Read`+`Edit`, because an
      agent that could rewrite its runbook could quietly lower the bar it is judged
      against. Round 2 then planted `playbooksDir: null` and every test stayed
      GREEN: two correct halves and nothing testing the join. `test/main/agents.test.ts`
      now spawns through a real `AgentManager` with a real `ClaudeAdapter` and reads
      the settings file spawn actually writes. Final round: 23 of 23 real mutants
      killed, both planted no-op controls survived.
      [Record](implementations/2026-09-09-m8b-1-install-playbooks.md).*
- [x] **M8b.2 A meeting with one attendee must be able to end** — Finding 11, the
      direct cause of SRS §6.1 clause 4 failing. The runner convened the meeting
      `ephctl help` documents and `EXIT-M8.md` §5.4 sends them to, and it could not
      terminate.
      *Evidence: a round rule plus routing, 130 tests green and mutants dying — then
      the adversarial pass asked whether anything TELLS an agent it may decline. It
      did not: the shipped `prompts/odeon/meeting-floor.md` said to "say that in one
      line", and saying it is a turn, and a turn resets the round. Artemis declared
      nothing further at seq 393 and again at seq 427 on 2026-09-09 and the floor came
      back both times; **the round rule alone would not have ended that meeting.** She
      reached `refuse` on her third attempt by reasoning about the log, not because
      anything named the act. The prompt now names it, says what a decline does, and
      says why a "nothing further" turn is not one — asserted against the SHIPPED file,
      not a fixture. 11 of 11 real mutants killed, control survived.
      [Record](implementations/2026-09-09-m8b-2-a-meeting-can-end.md).*
- [x] **M8b.3 A brief that is archived must exist** — Finding 13, **which is wrong on
      its central claim, and correcting the record is half the package.** The finding
      reported at high severity that the run's one brief was archived pointing at a
      file that was never written. The file was written; the ref could not be resolved
      from the home a reader actually has.
      *Evidence: refs now carry their root rather than assuming an agora-relative one.
      6 of 6 real mutants killed — including P6 (`minutesRef` reverting to an
      agora-relative ref), which **survived the first round** and was killed only after
      a test was written for it. P1 reproduces the 2026-09-09 state exactly.
      [Record](implementations/2026-09-09-m8b-3-refs-resolve.md).*
- [x] **M8b.4 The Odeon endpoint and the orchestrator agree on their vocabulary** —
      Finding 12. Artemis's first attempt to end a meeting was rejected outright at
      seq 439–440 because a shipped prompt instructed an act the endpoint refuses.
      *Evidence: 5 of 5 real mutants killed and the Q6 control survived, as it must.
      Q4 is the one that matters — it makes a **shipped prompt** instruct the act that
      bounced on 2026-09-09; Q5 makes a shipped prompt name an endpoint that does not
      exist. Both die.
      [Record](implementations/2026-09-09-m8b-4-odeon-vocabulary.md).*
- [x] **M8b.5 The ledger's first refusal must teach, or not happen** — Finding 6 as
      corrected in §4a of that record. Every incident's first task-open was refused,
      and the refusal did not carry enough for the agent to learn the rule — so it
      was billed for the same mistake repeatedly.
      *Evidence: a refusal now reports every issue rather than only the first, carries
      the validator's own words, and uses the caller's path shape rather than the
      validator's internal `ops.0`. 8 of 8 real mutants killed; R1 restores the
      2026-09-09 behaviour and dies, R5 makes a broken proposal answer with an uncapped
      wall of reasons and dies.
      [Record](implementations/2026-09-09-m8b-5-ledger-refusal-teaches.md).*
- [ ] **M8b exit** — `docs/EXIT-M8.md` re-run end to end by a runner who is not the
      author, from a clean clone. **NOT MET.** A rehearsal was performed on 2026-09-09
      ([record](demo/m8b-rehearsal-m8b-rehearsal.md)) and produced four findings rather
      than a pass: **A** (the crew could not be brought back after the §4 restart) and
      **B** (the exit script dictated a commit message telling the crew the break was
      deliberate) became M8c.9 and M8c.10; **C** re-confirmed Findings 3 and 4 still
      live; **D** upgraded Finding 10. A rehearsal that finds defects is the mechanism
      working — it is not the criterion. **This box and the M8c exit box below are the
      same run.**

## M8c — Bounded, and honest about itself (plan drafted 2026-09-09 at M8b close)

Derived from IMPLEMENTATION M8c + the remaining findings of
[the M8 exit run](demo/m8-onehour-aftershock.md) + the four findings of
[the M8b rehearsal](demo/m8b-rehearsal-m8b-rehearsal.md). M8b gave the crew the
ability to act. This milestone is about the two things that made the first exit run
unaffordable and its own report untrustworthy: **nothing bounded the spend, and the
harness described itself in a vocabulary its reader did not share.** Packages are
listed in number order; they were built in the order 10, 1, 8, 2, 3, 4, 5, 6, 7, 9,
3b, which the suite totals in each evidence note record.

- [x] **M8c.1 A ceiling must be reachable without a mouse** — Finding 3, confirmed
      still live by the M8b rehearsal's Finding C. `EXIT-M8.md` §2 is titled *"Set a
      ceiling before you walk away"* and calls itself *"the step that is skipped and
      then regretted"* — and then instructed **WATCH → settings**, which a runner who
      is not a person at the machine cannot reach. Both runs so far went out
      unbudgeted for exactly this reason.
      *Evidence: `ephctl budget:set --daily` writes the same file the panel does. The
      verb may only ever LOWER the ceiling — raising one is refused by name, with the
      reason and where to do it instead, because tightening a cap is a script's to do
      and loosening it is a person's (ADR-0033). Test Files 234 passed · Tests 4500
      passed, 8 skipped (4508). Mutation round: 10 of 10 real killed, control
      survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-1-budget-verb.md).*
- [x] **M8c.2 The first ingest must not replay history as news** — Finding 5.
      Activation completed at 09:06:36; by ~09:08, **before the runner had broken
      anything**, the Harbor's first ingest had pulled ten CI runs and the crew had
      raised eight incidents against two weeks of already-settled history.
      *Evidence: the first ingest now reads history as history. Test Files 235 passed ·
      Tests 4524 passed, 8 skipped (4532). Mutation round: 10 of 10 real killed,
      control survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-2-cold-start.md).*
- [x] **M8c.3 Findings 3 and 5 compound, and that is the lesson** — the cost-control
      package, and the reason this milestone exists. In the filing's own words:
      *"No ceiling could be set **and** the cold start replayed two weeks of history.
      Neither alone is alarming; together they turned 'walk away for an hour' into
      40,453,419 tokens ($11.22)"* — with the harness projecting 72.3% of the
      five-hour window consumed.
      *Evidence: the consent grant now asks for the ceiling rather than assuming one.
      Test Files 235 passed · Tests 4536 passed, 8 skipped (4544). Mutation round:
      10 of 10 real killed, control survived, ROUND OK. Three gaps the round surfaced
      were closed in `9e1aaf8` before the merge.
      [Record](implementations/2026-09-10-m8c-3-consent-asks-the-ceiling.md).*
- [x] **M8c.3b One suggested ceiling, and a refusal that reads once** — *not planned;
      **found by running the M8c rehearsal**, five minutes in, before any agent had
      been hired.* `EXIT-M8.md` §2 sends the runner to `consent:grant`, and the
      refusal came back `consent was NOT granted: consent was NOT granted: there is
      no daily token ceiling…`. Two defects in one line, both introduced that same
      morning: the prefix was added by both `ControlServer` (`control.ts:535`) and
      M8c.3's `grant()`, and the suggested figure still read 300,000 after the
      EXIT-M8 §2 correction had raised it to 5,000,000. **A refusal that stutters is
      a refusal a reader stops reading — the exact failure mode the "a refusal must
      teach the rule" standard warns about, introduced by the package that was
      fixing it.**
      *Evidence: Test Files 236 passed · Tests 4607 passed, 8 skipped (4615).
      Mutation round: 7 of 7 real killed, ROUND OK. Merged as PR #57.
      [Record](implementations/2026-09-10-m8c-3b-one-suggested-ceiling.md).*
- [x] **M8c.4 `WORKING` must cite a row that proves completion** — Finding 7, and the
      sharpest form of the defect M8.13 was built to prevent. At 06:12:41Z, with eight
      ledger refusals already in the log and **zero tasks succeeded**, `DIAGNOSIS.md`
      reported `incidents | WORKING | profile/incident-raised at seq 85`. Seq 85 is
      real and was quoted honestly — but `incident-raised` proves only that an incident
      was raised, not that it was routed, triaged, actioned or even recorded as a task.
      Five rows later the ledger refused it, and seven siblings after that.
      *Evidence: an area may now cite only a row that proves completion, not entry.
      Test Files 235 passed · Tests 4547 passed, 8 skipped (4555). Mutation round:
      10 of 10 real killed, control survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-4-working-cites-completion.md).*
- [x] **M8c.5 A deduplicated condition must not lose what distinguishes its
      occurrences** — Finding 2, confirmed still live by the M8b rehearsal's Finding C.
      `DIAGNOSIS.md` reported `home/seeded-config — authority.json was missing and has
      been created with the shipped default (×2)`, while `log.jsonl` seq 1 recorded the
      same cause naming `gate-policy.json`. The dedup key collapsed two different files
      into one row and kept only one filename, so the reader was told to review a file
      that was not the one seeded.
      *Evidence: Test Files 235 passed · Tests 4558 passed, 8 skipped (4566). Mutation
      round: 6 of 6 real killed, control survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-5-seeded-config-names-the-file.md).*
- [x] **M8c.6 Labels and docs that are true in the reader's vocabulary** — Findings 4
      and 1, and **Finding 4 caught a second runner at the M8b rehearsal.**
      `profile:activate` and `profile:instances` both reported `armed` listing only the
      two sweep triggers and no `ci` trigger — while `EXIT-M8.md` §5.1 says
      unambiguously that no bound `ci` trigger is a setup defect and *"the run cannot
      proceed past it"*. **The documented reading of that output was therefore: stop,
      the run is invalid.** It was bound; `profiles/skeleton-crew/triggers/ci-failure.json`
      declares it. `armed` meant "has a clock", and only scheduled triggers have one.
      *Evidence: `armed` now says which, in the reader's vocabulary. Test Files 235
      passed · Tests 4564 passed, 8 skipped (4572). Mutation round: 8 of 8 real killed,
      control survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-6-armed-says-which.md).*
- [x] **M8c.7 Recall must fail fast or not accept the call** — Finding 9, in the
      health-watcher's own words: *"`$EPH_RECALL` is **unavailable, not merely empty**.
      Two attempts (unscoped, and `--scope knowledge`) produced **zero bytes of output
      and never terminated**"*. A hang is now a reported condition rather than a
      silence.
      *Evidence: Test Files 236 passed · Tests 4588 passed, 8 skipped (4596). Mutation
      round: 9 of 9 real killed, control survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-7-recall-fails-fast.md).*
- [x] **M8c.8 Decide what an engine-level permission prompt is** — Finding 10,
      upgraded by the M8b rehearsal's Finding D. The harness records `gate/ungated ·
      tool-permission · waiting · "Claude is waiting for your input"` whenever an agent
      meets its engine's own permission prompt — and during an unattended hour there is
      nobody to answer it, so the agent simply stops. A hire now declares in advance
      what it may do without asking.
      *Evidence: Test Files 235 passed · Tests 4508 passed, 8 skipped (4516). Mutation
      round: **fourteen** real mutants and a planted no-op — 14 of 14 real killed,
      control survived, ROUND OK.
      [Record](implementations/2026-09-10-m8c-8-engine-prompts.md).*
- [x] **M8c.9 A crew must be able to come back after a restart** — the M8b rehearsal's
      Finding A. After the `EXIT-M8.md` §4 force-kill the instance restored correctly —
      plan back, trigger clock back, `seq` contiguous, consent not re-asked — and then
      **the crew could not be brought back by any documented surface.** A restart that
      restores everything except the workers is a restart that ends the run.
      *Evidence: the acceptance test runs a **second** harness against the home the
      first one left and spawns the same agent, which is the shape the defect had.
      Mutation round with a control: nine real mutants and a planted no-op. The
      adversarial pass then walked restart → reactivate looking for a second blocker the
      spawn seam might hide — `installPlaybooks` replaces idempotently, `beforeHires`
      cannot refuse, `targetExists` still holds, `activate` takes over a `down`
      instance, triggers re-arm from `armed: []` — and none of them refuses.
      [Record](implementations/2026-09-10-m8c-9-crew-comes-back.md).*
- [x] **M8c.10 The exit script must not tell the crew the break is deliberate** — the
      M8b rehearsal's Finding B. `EXIT-M8.md` §3 dictated the commit message for the
      deliberate break the entire exit run is built around — so the crew was handed the
      answer to the question it was being measured on. The planted break now reads like
      an ordinary change.
      *Evidence: Test Files 234 passed · Tests 4484 passed, 8 skipped (4492). Mutation
      round: 9 of 9 real killed, control survived, ROUND OK — the control being
      `docs/DECISIONS-LOG`, with a case asserting the planted break is caught.
      [Record](implementations/2026-09-09-m8c-10-exit-break-message.md).*
- [ ] **M8c exit** — an unattended run of `docs/EXIT-M8.md` completing inside a
      **stated** ceiling, with: no incident raised for a run older than the activation;
      `DIAGNOSIS.md` reporting no area as `WORKING` on the strength of an entry row,
      verified by planting an entered-but-failing pipeline and reading the report; a
      restarted company bringing its crew back with no manual filesystem step; and the
      hour ending because **the work ended** rather than because an agent met a prompt.
      **NOT MET, and never carried to completion.** No run artifact of any kind exists
      under `docs/demo/`. The repository's only durable trace of an attempt is M8c.3b,
      which records being found *"five minutes in, before any agent had been hired"*. A
      later attempt is remembered as aborted seconds into the hour when the machine
      moved; **that is a recollection with no record in the tree, and is logged here as
      owed rather than as evidence.**
      **This box is also the M8b exit above, and M7b sits behind it.** The four packages
      built to make the run survivable at all — M8c.1 (a ceiling without a mouse),
      M8c.2 (no history replayed as news), M8c.8 (no unanswerable prompt) and M8c.9
      (the crew comes back) — are all landed, so the run is unblocked for the first
      time.

### M8b / M8c record note (2026-09-15)

These two sections were written on 2026-09-15 from the sixteen implementation docs and
their merge commits on `origin/main` @ `fe8d4a2`, **nine days after the fact**. Between
2026-09-06 and this entry PROGRESS.md carried no mention of either milestone while
sixteen packages landed — so a session resuming at "the first unchecked box" would have
restarted at M6.9 or M7b. Every figure above is quoted from the package's own record;
**nothing was re-run to produce this entry**, and no box was ticked that its
implementation doc did not already evidence. See
[the status snapshot](status/2026-09-15-status.md).

## The Rust rewrite — R0 to R14 (ADR-0041; plan approved 2026-10-19)

The order and each phase's exit criteria are in [`RUST-PLAN.md`](./RUST-PLAN.md) §5.
A phase's packages are listed here when the Architect approves them at that phase's
start. R0's were approved with the plan. A ticked box carries its evidence, the same
as every M package.

### R0 — The workspace and its gates

- [x] **R0.1 Workspace skeleton** — `Cargo.toml`, `rust-toolchain.toml` (1.96.0),
  `[workspace.lints]`, `clippy.toml`, `deny.toml`, `target/` ignored, and
  `CARGO_TARGET_DIR` outside OneDrive documented. *Evidence (2026-10-19, win32,
  toolchain 1.96.0):* `cargo xtask gate` all steps green (9 unit + 5 integration tests;
  cargo-deny advisories, bans, licenses, sources ok). 13 of 13 planted lint violations
  refused by their own rule. An MPL-2.0 crate and a `*` version were each refused by
  cargo-deny. Mutation round: 31 mutants, 26 caught, 5 unviable, 0 missed. Its control
  (unit tests only) reports 12 missed. A stale `Cargo.lock` refused (exit 101, lockfile
  unchanged) after a review found the first alias rewrote it and reported green. Linux
  and Windows CI not yet run (R0.2).
  [`2026-10-19-r0-1-workspace-skeleton.md`](./implementations/2026-10-19-r0-1-workspace-skeleton.md)
- [x] **R0.2 The gate chain** — `xtask` with `gate`, `invariants` and `deps`, and the
  `rust` CI job on Linux and Windows. Each rule fails a planted violation by file and
  line. *Evidence (2026-10-19):* gate all six steps green on win32 (76 unit + 8
  integration tests), and fmt, clippy, tests, invariants and deps green on Linux (WSL2).
  Every rule refused its plants by file and line in a scratch workspace: 10
  invariants violations and 3 deps plants, offline, lockfile unchanged. `cargo xtask
  mutants xtask` round 3: ROUND OK, 225 mutants, 211 caught, 0 missed, 14 unviable;
  control 55/55 missed; sources unchanged. Rounds 1 (34 survivors, each answered) and
  2 (baseline failed in the copy) are in the doc. CI run 39312750826 at `970ac0b`: the
  Rust gate succeeded on ubuntu, windows and macos, and every TypeScript check passed.
  [`2026-10-19-r0-2-the-gate-chain.md`](./implementations/2026-10-19-r0-2-the-gate-chain.md)
- [x] **R0.3 The standards learn Rust** — the Rust edition of ENGINEERING-STANDARDS and
  BUILD-PROMPT §3 (ADR-0041's table), `check-readme-current.cjs` learning the `R`
  prefix, and the skills learning the R-order and the Rust gate. *Evidence
  (2026-10-19):* see [`2026-10-19-r0-3-standards-learn-rust.md`](./implementations/2026-10-19-r0-3-standards-learn-rust.md)
  §6. The README check sees the R track and named R0.1 and R0.2 as undescribed on the
  real tree before the README learned them. Its four new cases pass, and its mutation
  round is in the doc. The SRS amendments to FR-1.1, NFR-8 and NFR-16 are applied.

**R0 verdict (2026-10-19): DONE.** RUST-PLAN §5's exit criteria, verified by execution:

| Criterion | Evidence | Tier |
|---|---|---|
| The chain is green on win32 | `cargo xtask gate` all six steps green at R0.2's and R0.3's heads | A |
| …and on CI on Linux and Windows (macOS too, by the Architect's matrix) | Run 39312750826 at `970ac0b` (R0.2) and run 39317331295 at `b88da94` (R0.3): `Rust gate` success on ubuntu, windows and macos; every TypeScript check passed | B |
| Each `xtask` rule fails a planted violation by file and line | R0.1: 13 of 13 lints. R0.2: 10 invariants violations and 3 deps plants on a real scratch workspace. R0.3: the README check named R0.1 and R0.2 on the real tree before the README described them | A |
| Gate-shaped changes carry a mutation round with a control | R0.1: 26 caught, 5 unviable, control 12 missed. R0.2 round 3: ROUND OK, 225 mutants, control 55/55. R0.3: `mutate.cjs` ROUND OK, 11 of 11, control survived | A |

Debt carried forward, none of it blocking:

- the skills' new text takes effect in a session only after R0.3 merges and the main
  checkout pulls;
- Rust coverage floors start with R1.1's first crate (Architect, 2026-10-19);
- R phases stay in this file, not `docs/progress/`, until the archive learns the
  cross-track order (deferred by the Architect, 2026-10-19).

Merged so far: R0.1 as #102 (`f484a33`). Open: R0.2 as #104. R0.3 is stacked on R0.2
and opens when #104 merges.

### R1 to R14

- [ ] **R1** The home and the book of record (package list approved 2026-10-19)
  - [x] **R1.0 The design** — the new SDD section for the home and every persisted
    schema, redesigned from first principles; one Architect approval before R1.1.
    *Evidence (2026-10-20):* `docs/sdd/SDD-RUST.md` §2 and §4, built from an inventory of
    every persisted file, with each of the 36 schema modules mapped to a row. Approved
    by the Architect with its §4.13 amendments, the cursor's removal, and SDD-RUST as the
    Rust tree's SDD (DECISIONS-LOG 2026-10-20).
    [`2026-10-20-r1-0-home-design.md`](./implementations/2026-10-20-r1-0-home-design.md)
  - [x] **R1.1 `eph-schema`** — the models to the approved design, and the first Rust
    coverage floors.
    *Evidence (2026-10-20):* `crates/eph-schema` types every format R1 needs, read
    through one strict path that refuses a key written twice. The six formats whose
    shape changed start at version 2. `cargo xtask coverage` is the gate's seventh step,
    over production lines only. Before the mutation rounds, a design-fidelity review
    found 17 findings: 15 were fixed, and 2 went to the Architect and were decided
    (version 2 plus a home marker in R1.2; the ledger is consistent as a whole, which
    constrains R1.4's retention). Mutation rounds with honest controls: `eph-schema`
    ROUND OK on round 3 (270 of 270); `xtask` 483 of 484 on round 2, its one survivor
    recorded with its reason. There were two refutation passes over the coverage check
    and the ratchet. Linux in WSL matched Windows figure for figure and hash for hash.
    The floors were seeded by `cargo xtask coverage ratchet` from CI run 39402429611's
    three attempts on all three platforms: `eph-schema` 98.86% and `xtask` 98.26%, on
    tree `fnv1a64:9c42f0f107e84a79`. CI run 39406446582 on the commit carrying the
    floors is green on every job, including all seven gate steps on all three platforms.
    [`2026-10-20-r1-1-eph-schema.md`](./implementations/2026-10-20-r1-1-eph-schema.md)
  - [x] **R1.2 `eph-fs`** — atomic writes, the append-only log, the home and its seeding.
    *Evidence (2026-10-20):* `crates/eph-fs` is the one crate that writes a file another
    process reads. `write_atomic` syncs a temporary file and renames it over the target,
    waiting out the two refusals Windows was measured to give (OS errors 5 and 32) over a
    fixed 500 ms schedule. `AppendLog` syncs every line (1.02 ms against 0.22 ms,
    measured), recovers `seq` at open, leaves a torn tail alone and returns a skipped
    rotation. `home::open` marks and seeds a missing path or an empty directory, and
    refuses anything else byte for byte untouched, including a TypeScript-era home (the
    Architect's three R1.2 decisions). Mutation rounds with controls: `eph-fs` round 1
    INVALID on 3 timeouts in a clock-bounded retry, which became the fixed schedule;
    round 2 ROUND OK (82 of 82 viable), one exclusion recorded with its reason;
    `eph-schema` ROUND OK (271 of 271). The floor was seeded by `cargo xtask coverage
    ratchet` from CI run 39418850906's three attempts on all three platforms: `eph-fs`
    97.07% (the Windows figure; POSIX runs the directory sync, 97.66%), and `eph-schema`
    raised to 98.87%, on tree `fnv1a64:9f6380dea039ffec`. CI run 39422475429 on the
    commit carrying the floor is green on every job, including all seven gate steps on
    all three platforms.
    [`2026-10-20-r1-2-eph-fs.md`](./implementations/2026-10-20-r1-2-eph-fs.md)
  - [x] **R1.3 `eph-git`** — the single committer and worktrees; hermetic git in every
    git-running test.
    *Evidence (2026-10-20):* `crates/eph-git` is the one crate that runs git. Its two
    Architect decisions:
    - the committer holds an OS lock on `.git/ephesus-committer.lock` for life, so a
      second committer is refused and a crash leaves no stale lock;
    - the Agora's git reads none of the machine's configuration.

    Open takes the lock before `git init --template=` and judges a killed harness's
    `index.lock` before any commit. Commits run on one queue thread, batched, retried over
    a fixed schedule, and a failure nobody waits for is reported. Worktrees identify
    repositories by git's common directory. They never delete uncommitted work, or a
    detached commit nothing holds.

    A design-fidelity review and a refutation pass found real defects, several measured:
    - XDG ignore files and a template reached the "hermetic" Agora;
    - `status.showUntrackedFiles=no` made `git worktree remove` delete work, which is
      also patched in the TypeScript build as `fix/worktree-untracked-files`.

    Three answers came from the Architect after review. Real kills, with the test binary
    rerun as the harness, at each fault point and at random, lose nothing on reopen, and
    fsck is clean.

    Mutation rounds with controls: `eph-git` ROUND OK on round 4 (103 of 103 viable),
    with one timing-only exclusion recorded, and `eph-fs` ROUND OK (83 of 83). CI found
    three more defects, all fixed: a lock a forked child still held on Linux, and two
    tests on Windows that claimed more than the design does.

    The floor was seeded from CI run 39465824068's three attempts on all three
    platforms, on tree `fnv1a64:26f0592f6e03d9d1`: `eph-git` 98.92% (Windows 99.05%), and
    `eph-fs` raised to 97.24%. CI run 39471410815 on `15b77f6`, the final tree, is green on
    every job, including all seven gate steps on all three platforms.
    [`2026-10-20-r1-3-eph-git.md`](./implementations/2026-10-20-r1-3-eph-git.md)
  - [x] **R1.4a `eph-agora`, the stores** — `Store<T>`, the registry, the task ledger and
    its retention, the board, seeding the Agora, the boot order (split from R1.4 by the
    Architect, 2026-10-20).
    *Evidence (2026-10-20 and 2026-10-25):* `crates/eph-agora` is the one store every
    structured record goes through. It reads a missing file as absent and anything else
    it cannot accept as damaged, and it never writes over damage. On it sit the roster,
    the ledger with its retention (past 9,000 tasks, done tasks beyond the newest 1,000
    move to an append-only archive, never one a kept task depends on), the board, and the
    boot. The boot renders `.gitignore` before the reconcile, renders `PROTOCOL.md`,
    seeds what is absent, and stops only when the committer refuses.

    A design-fidelity review and a refutation pass ran before any round (fixed in
    `40c1fe6`). Ten Architect answers followed on 2026-10-25, among them SDD-RUST §10
    approved, four departures from the written design ratified, a seq-less archive
    writer, and the fix below.

    The Linux run (WSL2) found two R1.3 tests racing and an R1.3 defect: a killed
    `git init`'s `config.lock` refused every later boot. Open now finishes the init and
    the boot reports it. On the final tree, plain and instrumented suites each failed
    0 of 12, and the four POSIX-only tests passed.

    Mutation rounds with honest controls, no TIMEOUT:
    - `eph-agora` ROUND OK (89 mutants, 49 caught, 0 missed), after a first round's two
      equivalent survivors became `append_line`;
    - `eph-git` ROUND OK (164, 112 caught);
    - `eph-fs` ROUND OK (101, 89 caught);
    - `eph-schema` ROUND OK (319, 276 caught).

    The floor was seeded by `cargo xtask coverage ratchet` from CI run 40165646997's three
    attempts on all three platforms, on tree `fnv1a64:81712cd8a87bd1c7`: `eph-agora`
    95.97% (Windows 96.64%). It raised `eph-fs` to 97.48%, `eph-git` to 98.96% and
    `eph-schema` to 98.92%, and lowered nothing. Each attempt was red at the coverage step
    alone, by design, and every TypeScript job was green.
    [`2026-10-25-r1-4a-agora-store.md`](./implementations/2026-10-25-r1-4a-agora-store.md)
  - [ ] **R1.4b `eph-agora`, the book of record** — the log writer and readers, publish and
    subscribe, the degradation channel, killed mid-append (`DIAGNOSIS.md` moved to R7).
- [ ] **R2** The terminal plane and the engine seam
- [ ] **R3** The event plane
- [ ] **R4** The first window
- [ ] **R5** Hermes
- [ ] **R6** The Watch
- [ ] **R7** Artemis, the control surface, the headless company
- [ ] **R8** The Library
- [ ] **R9** The Odeon, the org and the profiles
- [ ] **R10** The Harbor
- [ ] **R11** The Gymnasium, the Stoa and company modes
- [ ] **R12** The Herald
- [ ] **R13** The floor and the full window
- [ ] **R14** Parity: the TypeScript tree deleted, the ruleset switched, M9 asked

---

## M9 — The harness is the product (plan drafted and APPROVED FOR PLANNING 2026-10-02; build NOT started)

Derived from the Architect's 2026-10-02 direction and recorded in full in
[`M9-PLAN.md`](./M9-PLAN.md) (thesis, evidence, packages with acceptance and tests,
exit, risks, the seven decisions). Digest in IMPLEMENTATION M9. **Inserted before
M7b** (DD-M9-1). The Architect approved *the documentation and planning*, in those
words, and said not to build yet — so every box below is unticked on purpose, and a
session resuming here starts by asking the Architect whether the build may begin,
not by starting it. ADR-0036 is accepted; ADR-0037/0038/0039 are owed by M9.0 and
M9.3.

Decisions recorded 2026-10-02 (DECISIONS-LOG, seven entries): native engine = own
loop as own CLI in TS (DD-M9-2); first provider = OpenAI-compatible local endpoint
over `fetch` (DD-M9-3); headless-first, nothing deleted (DD-M9-4); a third TS build
target for `eph-agent` (DD-M9-5); Odysseus registered and Hermes Agent's tags amended
on the watchlist (DD-M9-6); bench T1 subsumes the unattended hour (DD-M9-7).

**Prerequisite landed 2026-10-03: GYM-008, the mutation round as a tool.** M9-PLAN §6
says every package here owes a round from a checked-in `test/mutation/<package>.json`,
and until today `test/mutation/` did not exist. `scripts/mutate.cjs` now runs one
(TEST-STRATEGY §10, ENGINEERING-STANDARDS §6 item 9). It was chosen ahead of M9.0 by the
Architect, and choosing it did not lift the hold: **whether the M9 build may start is
still the Architect's question**, and no box below is ticked.
[Record](implementations/2026-10-03-gym-008-mutation-harness.md).

- [ ] **M9.0 The contract** — ADR-0037, ADR-0038; SRS FR-15/FR-16/§6.11 and SDD
      §13 were drafted at plan time and are refined here; TEST-STRATEGY S-NATIVE and
      S-BENCH; `/research` run against `src-odysseus` and `src-hermes-agent` at
      pinned commits → RB-002, RB-003; M9-PLAN §3 replaced by citations.
      *Docs: M9-PLAN §6. Evidence owed: the two briefs validated (uncited finding
      refused), ADR append-only check green.*
- [ ] **M9.1 The bench, v0** — `bench/tasks/*.json`, runner over the control
      surface only, scorer over `log.jsonl` + cost ledger, `docs/bench/LEDGER.md`;
      S-BENCH discrimination in CI; one live baseline row on wrapped `claude`.
      *Evidence owed: S-BENCH output; the baseline row with its full condition.*
- [ ] **M9.2 `eph-agent`: the native engine** — `src/engine/` (third TS build,
      DD-M9-5), provider seam + OpenAI-compatible local provider, native hook
      emission, Stop question over the hook endpoint, JSONL transcript, refusals by
      name (endpoint, tool calls, context floor), stall guard; `native.ts`
      registered and in the conformance table.
      *Evidence owed: conformance table green for four subjects; S-NATIVE output;
      mutation round from `test/mutation/m9-2.json` with a control.*
- [ ] **M9.3 Tools, and the gate as the permission prompt** — closed tool registry,
      containment, `shell` grants as allowlist, native gate kind (ADR-0039), tool
      results tagged untrusted with provenance.
      *Evidence owed: S-GATE native cases; API-surface containment test; the
      adversarial refutation pass recorded with what it tried.*
- [ ] **M9.4 Memory and context** — Library layer, `recall` tool, compaction as
      `compacting` with byte-for-byte identity/protocol preservation.
- [ ] **M9.5 Claude, natively** — Anthropic Messages API over `fetch`, broker key,
      provider conformance against recorded fixtures; S-SECRETS for the key.
- [ ] **M9.6 The first uplift** — ablation switches, N ≥ 3 per cell on one local
      model and `claude` native, ledger rows, README section quoting the ledger,
      mode-gated weekly cadence.
- [ ] **M9.7 Headless-first** — `npm run headless`; reachability from the headless
      entry reaches zero renderer code; T1 runs end to end with no window.
- [ ] **M9.8 Exit review** — M9-PLAN §7, by `/milestone-review`; PROGRESS updated
      in the same session; a `docs/status/` snapshot.
- [ ] **M9 exit** — the six criteria of M9-PLAN §7, each with tier A/B evidence.

## Phase 9, continued — M9b to M9f: the agentic harness, to v1 (plan APPROVED FOR PLANNING 2026-10-03; build NOT started)

Derived from the Architect's direction of 2026-10-03 — the feature sets of the two
harnesses on the Stoa watchlist as Ephesus's target, the brand core kept, the floor
an optional view mode — and recorded in full in
[`PHASE-9-PLAN.md`](./PHASE-9-PLAN.md): the sixteen features mapped to their
subsystems, the rules every capability enters under, every package with its
acceptance, tests and risk, the bench tasks, the dependency memos expected, and the
decisions. Digest in IMPLEMENTATION. **M9 above is unchanged and comes first. Five
milestones follow it and precede M7b — this section. Two more, M9g and M9h, follow
M7b and are registered below it** (DD-M9-8, DD-M9-18).

The Architect approved the plan *for planning* on 2026-10-03, in the same words he
used for M9: the documentation stands as the project's plan, and nothing is built.

**Every box below is unticked on purpose.** The standing instruction is to plan and
not to build: a session resuming here asks the Architect whether the build may
begin — M9's first — and does not start it. Each milestone opens with a contract
package that writes its ADRs, SRS group, SDD, THREAT-MODEL and TEST-STRATEGY
sections and runs one governed `/research` cycle at a pin; the questions each
contract asks the Architect are listed in PHASE-9-PLAN §11 and are asked then, not
assumed now.

Decisions recorded 2026-10-03 (DECISIONS-LOG): follow-on milestones, M9 untouched
(DD-M9-8); a zero-dependency core and a memo per package after it (DD-M9-9);
features before M7b, M7b.4 absorbed (DD-M9-10); workspace by default, the floor a
view mode — ADR-0040 (DD-M9-11); web reach as a per-hire grant, and fetched content
arms the gate (DD-M9-12); one company-wide MCP registry (DD-M9-13); the loop stays
local and tools may run in a declared backend — SRS §1.2 amended (DD-M9-14); the
phone served first the way Hermes Agent does it — the gateway to that standard
and the workspace in a browser — and then by Ephesus's own companion, a milestone
of its own, designed before it is built (DD-M9-15); speech as output only, the
rest of M6.9 still deferred (DD-M9-16); both watchlist rows' tags amended
(DD-M9-17); and, asked again once the estimate was known to be roughly double what
he had been quoted, **M7b after M9f** — v1 is M9 through M9f, with M9g and M9h
after it (DD-M9-18, amending DD-M9-10).

### M9b — The workspace: a harness you talk to

- [ ] **M9b.0 The contract** — the conversation and skills ADRs; the SRS group;
      SDD; the UI-DESIGN §4 amendment; S-WORKSPACE, S-SKILL; T-SKILL and the
      `skills` switch; one `/research` cycle per source at a pin. Asks: Artemis's
      default engine.
      *Evidence owed: the briefs validated; ADR append-only check green.*
- [ ] **M9b.1 View modes** — ADR-0040 built: `view` in `config.json`, absent means
      `workspace`; the floor not loaded unless chosen; nothing deleted.
      *Evidence owed: the module-graph assertion; the floor suites green in CI; the
      upgrade-path test on a home written before the field existed.*
- [ ] **M9b.2 The conversation** — a pure projection of the transcript; input down
      the existing command path; gates inline through the one handler; a wrapped
      hire shows its terminal; `ephctl converse`.
      *Evidence owed: the projection suite; one spy, two callers; model output
      rendered inert; mutation round with a control.*
- [ ] **M9b.3 Sessions, and choosing a model** — the `companion` template; the
      model picker over capability records; resume; session search.
- [ ] **M9b.4 Attachments** — validated in main; the vision refusal.
- [ ] **M9b.5 Skills** — `SKILL.md`; inspect/install split; granted by name;
      propose through the Gymnasium; no agent write path under `tools/`.
      *Evidence owed: S-SKILL; the API-surface test; T-SKILL rows with `skills` on
      and off; the adversarial pass recorded with what it tried.*
- [ ] **M9b.6 The Architect's profile, and notes** — budgeted `architect.md`; the
      over-budget error; quick capture into the knowledge shelf.
- [ ] **M9b.7 Exit review** — PHASE-9-PLAN §6, M9b's exit, by `/milestone-review`.
- [ ] **M9b exit** — the six criteria, each with tier A/B evidence.

### M9c — The hearth: local models, first-class

- [ ] **M9c.0 The contract** — the dependency-class ADR for weights and servers;
      the judged-rows ADR; S-MODELS, S-ARENA. Asks: a city name for the service, or
      none; whether a script may start a pull, or only the window may.
- [ ] **M9c.1 What the machine is** — hardware facts with provenance; `unknown`
      rather than a guess.
      *Evidence owed: every parser over a recorded capture with its provenance.*
- [ ] **M9c.2 The catalogue, and what fits** — the pure fit function; the formula
      and its constants in the implementation doc.
- [ ] **M9c.3 Downloads** — the Architect's act only; checksum before the final
      name; the disk ceiling.
      *Evidence owed: the API-surface test that no agent path pulls; the
      adversarial pass.*
- [ ] **M9c.4 Serving, and the verified capability** — `owned`, `declared`,
      `attached`; the probed capability record; measured throughput in the bench
      condition.
      *Evidence owed: which kinds ran on which platform, and which are NOT
      EXERCISED.*
- [ ] **M9c.5 Compare** — the blind verdict as a judged row; the mapping in main
      until the verdict.
      *Evidence owed: S-ARENA; the scripted verdict refused by name.*
- [ ] **M9c.6 The council** — the `consult` grant; the `council` switch measured.
- [ ] **M9c.7 Exit review.**
- [ ] **M9c exit** — the five criteria, each with tier A/B evidence.

### M9d — The walls and the hands

- [ ] **M9d.0 The contract** — the backends, armed-gate, MCP and delegation ADRs;
      THREAT-MODEL §3/§5/§6.7 rewritten; the backend conformance table; S-TAINT,
      S-MCP, S-DELEGATE; T-INJECT, T-FANOUT. Asks: NFR-17 against the MCP registry;
      the company-wide backend floor; what arms the gate beyond fetched content
      (mail, MCP results, attachments, other people's messages), that a target
      repository's content does not, and whether a backend with no egress and no
      secret grant lifts it; SRS C-2 against remote tool execution.
- [ ] **M9d.1 The backend seam, `local` and `docker`** — the loop never moves; a
      declared isolation grade checked both ways.
      *Evidence owed: the conformance table output per backend; the adversarial
      pass.*
- [ ] **M9d.2 `apptainer`, `ssh`, `modal`** — the same table; pinned host keys;
      Modal's memo.
- [ ] **M9d.3 Provenance arms the gate** — S-TAINT; T-INJECT with `armedGate` on
      and off.
      *Evidence owed: zero canary leaks armed; the count of gates opened; the
      adversarial pass.*
- [ ] **M9d.4 MCP** — the company-wide registry; the harness as the only client;
      manifest drift refused; the outside-agent skill over `ephctl`.
      *Evidence owed: S-MCP; the S-SECRETS extension.*
- [ ] **M9d.5 Delegation** — ephemeral workers; teardown proven; T-FANOUT.
- [ ] **M9d.6 Scripted tool calls** — the same gate path inside a script; the caps.
- [ ] **M9d.7 Exit review.**
- [ ] **M9d exit** — the five criteria, each with tier A/B evidence.

### M9e — The open web and the research desk

- [ ] **M9e.0 The contract** — the web-reach and inquiries ADRs; NFR-10 amended;
      THREAT-MODEL §4; S-WEB, S-INQUIRY, S-DOCS, E-RESEARCH; T-RESEARCH. Asks: the
      browser against the Electron pin; owned or attached search.
- [ ] **M9e.1 Search and fetch** — the `web` grant; the address-refusal table; the
      content-addressed archive.
      *Evidence owed: S-WEB; the egress audit over a full scenario run.*
- [ ] **M9e.2 The browser** — offscreen, per-hire profile; input on an unlisted
      origin is a gate; inside its support window or NOT BUILT.
- [ ] **M9e.3 Sight, images, voice** — `vision`, `image_generate`; `speak` and
      spoken briefs — the Herald's output only (DD-M9-16); no input path of the
      Herald gains a caller.
      *Evidence owed: the reachability walk listing exactly what gained a caller.*
- [ ] **M9e.4 Inquiries** — one report per question; uncited or unresolvable
      findings rejected before a human; T-RESEARCH.
- [ ] **M9e.5 Documents** — an AI edit is a patch; no agent write path; the
      sandboxed HTML preview.
- [ ] **M9e.6 Exit review.**
- [ ] **M9e exit** — the five criteria, each with tier A/B evidence.

### M9f — The harbor opens: every surface, one Artemis

- [ ] **M9f.0 The contract** — the gateway, remote-authorisation and schedules
      ADRs; FR-10.2 rewritten additively; the THREAT-MODEL section before the first
      adapter; the platform conformance table; S-GATEWAY, S-REMOTE-APPROVE,
      S-SCHEDULE. Asks: remote memo verdicts; a PIN; the platform order, and whether
      FR-10.2's "Slack-compatible" still names the first.
- [ ] **M9f.1 The gateway, the terminal and the first platform** — `harbor/bridge.ts`;
      pairing by a code issued at the machine — single-use, expiring,
      attempt-limited, with a lockout; silence to the unpaired.
- [ ] **M9f.2 More platforms** — Discord, Slack, Signal, WhatsApp; one adapter per
      PR; an adapter not built is listed with its reason.
- [ ] **M9f.3 Remote authorisation** — the same validated path a click takes;
      nonce, digest, repeat-back; absent policy means off.
      *Evidence owed: S-REMOTE-APPROVE; the adversarial pass recorded with what it
      tried.*
- [ ] **M9f.4 Schedules from language** — the confirmed plan armed verbatim; the
      calendar-time trigger kind; fresh session per run; the pause; T-SCHEDULE.
- [ ] **M9f.5 Push, reminders and the morning brief** — ntfy; redaction by server
      kind.
- [ ] **M9f.6 The workspace in a browser** — a third front door onto the same
      handlers; loopback only; never without a session; a remote surface, not a
      second window.
      *Evidence owed: S-BROWSER-SURFACE; the machine-only acts refused by name
      over this transport, both directions; the adversarial pass.*
- [ ] **M9f.7 Exit review.**
- [ ] **M9f exit** — the six criteria, each with tier A/B evidence. **Carries the
      clause moved from M7b:** a truthful morning brief on the phone after an
      unattended night.

## M7b — The recursive company + shipping (plan drafted 2026-08-29 at M6 close)

> **Follows M9f (2026-10-03, DD-M9-18, amending DD-M9-10).** v1 is M9 through M9f;
> M9g and M9h are registered below this milestone and built after it. M7b.4 is
> absorbed by M9f.1–M9f.3 and is no longer a box of this milestone; the
> morning-brief clause of M7b.6 and of the exit moves to M9f's exit. Everything else
> below stands.

Derived from IMPLEMENTATION M7's inward half + ADR-0018 + ADR-0019 + ADR-0020 +
SRS FR-9.5/FR-10.2/FR-10.5 + UC-11/UC-16 + SDD §7.8 + TEST-STRATEGY §3
(S-RECURSE). This is the milestone where the company's primary standing mission
— improving itself — becomes an activatable bundle, and where the harness first
acts on the internet under an identity of its own. It is deliberately LAST:
everything it composes must already work.

- [ ] **M7b.1 Company GitHub identity** — ADR-0020: one Architect-owned machine
      account; a fine-grained PAT (contents + pull-requests on the named repo
      only) held WRITE-ONLY in the broker and env-injected at spawn solely to
      roles whose hire template declares the grant; agent commits authored as
      the company account with a per-agent `Co-authored-by:` trailer, never as
      the Architect and never as any vendor identity; the account holds write,
      never admin, with `main` PR-and-review protected so the HOST enforces
      what the harness promises. **`check-attribution.cjs` gains its carve-out
      here** — company-account authorship is legal only on `agent/*` branches,
      and a company-account commit on `main` that did not arrive by an
      Architect-merged PR fails the job (ADR-0020 names this as owed to the M7
      package that first exercises the identity).
      *Docs: ADR-0020 (normative), FR-10.5, ENGINEERING-STANDARDS §2. Tests:
      the carve-out asserted in BOTH directions — an `agent/*` company commit
      passes, the same commit on `main` fails, the Architect's own commits are
      unchanged, and a vendor identity anywhere still fails; the token reaches
      the improver role and NOT the researcher (NFR-17); revoking it disables
      delivery and nothing else. Risk: this loosens a rule that has held for
      every commit in the repository — the carve-out must be narrower than the
      thing it permits, and its test must fail if it widens.*
- [ ] **M7b.2 Recursive Improvement profile** — FR-9.5/ADR-0019 as the third
      built-in bundle: a researcher role running the Stoa cadence over the
      watchlist, improver role(s) implementing approved proposals in isolated
      worktrees, Artemis's ranking/pre-screen duties, and delivery playbooks;
      default target the company's own repository. **Activation is refused
      outside company mode `improving`**, naming §6.9's missing evidence; a
      mode revert or deactivation disarms its triggers. It runs inside the
      FR-12.5 gym budget slice and can never starve a coexisting profile.
      *Docs: ADR-0019 (normative), FR-9.5, ADR-0018, SDD §7.8. Tests: the
      refusal in `directed` names the missing evidence; a mode revert disarms
      triggers observably; the researcher's spawn plan carries no secret grants
      and a read-only checkout while the improver's carries the GitHub grant;
      the budget slice holds under contention. Risk: this profile modifies the
      harness that runs it — the strictest gate posture in the fleet, and no
      path in the bundle may widen it.*
- [ ] **M7b.3 PR delivery + the provenance chain** — SDD §7.8's arrow: approved
      proposal → improver's worktree → `agent/<name>/<topic>` branch → PR
      opened under the company identity via `harbor/github.ts`, its body citing
      the `GYM-<NNN>` and `RB-<NNN>` ids it descends from, logged `remote`-
      tagged → the Architect's queue. Rejection revises on the SAME branch; the
      proposal stays `approved`, never silently abandoned. **No auto-merge path
      exists anywhere in the profile or the code.**
      *Docs: SDD §7.8, ADR-0019, UC-16 incl. Alternate 5a. Tests: the absence
      of a merge/push-`main` path asserted by API SURFACE (the S-SECRETS
      pattern), not by a happy-path test; the PR body's citations are
      mechanically required, so an uncited PR cannot be opened; the full chain
      watchlist→brief→proposal→PR→merge→measured is reconstructible from the
      ledger and log alone. Risk: "the Architect merges" must be true because
      no code CAN merge, not because no code currently does.*
- **M7b.4 Chat bridge — MOVED to M9f.1–M9f.3 on 2026-10-03 (DD-M9-10); no longer
      a box of this milestone, and not built here.** *Its original text is kept
      for the record, and its tests and its risk note are inherited by M9f:*
      FR-10.2/UC-11: one Slack-compatible webhook/bot
      through which the Architect converses with Artemis remotely, receives
      briefings, and approves gates; inbound webhooks may spawn ephemeral
      workers torn down after replying. Every remote directive is echoed in the
      desktop activity log with the `remote` source tag (FR-10.3).
      *Docs: FR-10.2/10.3, UC-11. Tests: a gate approved over the bridge takes
      the SAME validated path a click takes (the ADR-0007 consequence, restated
      for chat); an ephemeral worker is provably torn down; inbound content is
      DATA, never instructions (invariant §13, NFR-17); the bridge being down
      is a visible degradation. Risk: the bridge is an inbound channel from
      outside the machine, it can approve gates, and it is the largest new
      attack surface in the project.*
- [ ] **M7b.5 Packaging and update check** — signed builds for macOS, Windows
      and Linux; a one-click update check. The licensed-art drop rule holds:
      what ships must not depend on sheets that are not redistributable.
      *Docs: IMPLEMENTATION M7, ATTRIBUTION. Tests: a build produced on each OS
      boots to its default view (*amended 2026-10-03 with ADR-0040, which makes the
      workspace that view; it read "boots to the floor"*); the update check is a visible, refusable prompt,
      never a silent self-update; no licensed asset ships that ATTRIBUTION does
      not permit. Risk: signing identities are secrets — the broker's rules
      apply to the build pipeline too.*
- [ ] **M7b.6 Suites + exit review** — S-RECURSE green in CI; **the recursive
      test (SRS §6.10) landing one REAL chain** — a URL on the Stoa panel →
      watchlist entry → brief citing it → approved proposal descending from it
      → company-identity PR on an `agent/` branch citing both ids → Architect
      merge; ~~a real overnight run producing a truthful morning brief ON THE
      PHONE~~ *(moved to M9f's exit, 2026-10-03, DD-M9-10)*; the Gymnasium and
      Stoa cadence triggers live under company-mode
      governance (ADR-0018 — autonomous only in `improving`); and the
      **two-week gymnasium acceptance test (SRS §6.7) BOOKED as the final v1
      acceptance gate**, its window recorded on the ledger the way the M6
      metric sweep was.
- [ ] **M7b exit** — SRS §6.10's real chain landed; S-RECURSE pass; ~~the
      overnight remote brief demonstrated~~ *(moved to M9f's exit, 2026-10-03)*;
      cadences live under mode governance;
      SRS §6.7 booked with a date; signed builds on three OSes; PROGRESS + docs
      synced. **This is the v1 acceptance boundary** — after it the only gate
      left is §6.7's two-week run.

## Phase 9, after v1 — M9g and M9h (planned 2026-10-03 with the rest of the phase; build NOT started)

**These two follow M7b** (DD-M9-18): v1 is M9 through M9f, and no v1 requirement
asks for mail, a calendar or a companion app. They sit below M7b on purpose: the
order of the boxes in this file is the order of the build. Packages, acceptance, tests and risk
are in [`PHASE-9-PLAN.md`](./PHASE-9-PLAN.md) §6. Every box is unticked on purpose.

### M9g — The Architect's desk: mail, calendar, todos

- [ ] **M9g.0 The contract** — the mail-and-calendar ADR; the THREAT-MODEL section
      for mail; S-MAIL, S-CAL; E-TRIAGE. Asks: whether inbound mail may instruct;
      the mail libraries.
- [ ] **M9g.1 The mail connector** — IMAP/SMTP; mail is untrusted; no send tool —
      the outbound gate sends.
- [ ] **M9g.2 The inbox profile** — a triager that cannot draft, a drafter that
      cannot send; E-TRIAGE, T-TRIAGE.
- [ ] **M9g.3 Calendar and todos** — CalDAV; the recurrence table; todos as ledger
      tasks.
- [ ] **M9g.4 Exit review.**
- [ ] **M9g exit** — the four criteria, each with tier A/B evidence.

### M9h — The companion: Ephesus on the phone, as its own subsystem

- [ ] **M9h.0 The design** — the companion ADR; the protocol in the SDD before any
      code; the THREAT-MODEL section with its cases named; S-COMPANION; one
      `/research` cycle. Asks: the client's form; reach away from home; the
      device's own unlock on an approval; the push channel.
      *Evidence owed: the adversarial review of the design AS A DOCUMENT, with
      what it tried, before M9h.1 starts.*
- [ ] **M9h.1 Device identity and pairing** — a key that never leaves the phone;
      pairing only at the machine; a code with no long-lived secret; a device
      list with no secret in it.
      *Evidence owed: S-COMPANION pairing cases; the adversarial pass.*
- [ ] **M9h.2 The channel** — device-authenticated, end to end at the application
      layer; replay, tampering and downgrade each refused; no primitive written
      by hand.
      *Evidence owed: the wire capture carrying no plaintext; test vectors for
      every primitive, with where they came from.*
- [ ] **M9h.3 The companion itself** — an installable client of the same
      handlers; nothing old shown as current; no secret, no mode, no registry.
- [ ] **M9h.4 Push to the device** — encrypted to the device, or wake-and-fetch.
- [ ] **M9h.5 Exit review** — and the phase's closing review.
- [ ] **M9h exit** — the seven criteria of PHASE-9-PLAN §6, each with tier A/B
      evidence; the last is the phase's closing review: every bench task with its
      deterministic half in CI.

---
name: build-package
description: Execute the next Ephesus work package from BUILD-PROMPT.md — resume from docs/PROGRESS.md, run the READ→PLAN→BUILD→TEST→PROVE→COMMIT→REPORT loop, and never proceed on red checks. Use when asked to continue building Ephesus, implement a milestone/package, or "keep going".
---

# Build the next work package

1. Read `BUILD-PROMPT.md` in full. It is the contract for this session; its §3
   invariants and §7 prohibitions override any instinct you have.
2. Open `docs/PROGRESS.md` and read its *Resume here* block: the package to build is
   the one it names, and where it says the build waits on the Architect, ask him first
   (AskUserQuestion) and stop on anything but a yes. If the milestone has no package
   list yet, derive one from its section of `docs/IMPLEMENTATION.md` and write it into
   PROGRESS.md before coding.
3. Read only the docs BUILD-PROMPT §2 maps to this package.
4. Execute the working loop (BUILD-PROMPT §4). Hard rules:
   - `npm run typecheck && npm run lint && node scripts/check-invariants.cjs && npm run test:coverage && node scripts/check-coverage.cjs` green before any commit — BUILD-PROMPT §4's TEST line (since M8.0 the suite runs once, under coverage, and the seam rule's two checks run with it).
   - Tests owed by the package (TEST-STRATEGY §2 mapping) are part of the package.
   - Evidence (command + observed result) goes in the commit description.
   - 3 failed distinct attempts on a blocker → stop, record it, report (don't hack around).
5. Tick the package in `docs/PROGRESS.md` with a one-line evidence note and update the
   *Resume here* block in the same commit; log any minor choices in
   `docs/DECISIONS-LOG.md`; queue must-ask questions for the Architect.
6. End with the session report format from BUILD-PROMPT §9.

Package too big mid-flight (> ~10 files)? Split it in PROGRESS.md and do the first half
properly rather than all of it loosely.

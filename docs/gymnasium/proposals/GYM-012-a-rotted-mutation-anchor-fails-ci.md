# GYM-012 — A rotted mutation anchor fails CI, as GYM-008 said it would

> **Carried to `main` on 2026-10-19; it never landed here.** The step below was built
> on `fix/gym-008-follow-up` (PR #77), which was closed unmerged when ADR-0041 froze the
> TypeScript tree. On `main` its status is *approved, superseded before landing* (the
> ledger row). cargo-mutants generates its mutants rather than reading hand-written
> anchors, so the failure this guards against has no counterpart in the Rust harness
> (`cargo xtask mutants`). The status line below is the branch's, kept as it was.

**Status:** landed (2026-10-11, `fix/gym-008-follow-up`; metric 1 measured at landing,
see the ledger) · **Proposed:** 2026-10-11 · **Decided:** 2026-10-11 (Architect,
asked with the drafted metric below before any code was written, during the GYM-008
follow-up) · **Gate:** Architect approval. It adds a step to the required
`Typecheck · lint · test` check, so it is an altered CI gate (ENGINEERING-STANDARDS §3;
DECISIONS-LOG 2026-09-02, "A changed CI gate gets a ledger entry"). It touches no
BUILD-PROMPT §3 invariant, no ADR, no secret and no dependency: the step runs
`scripts/mutate.cjs`, already in the tree, on the node the job already installs.

**Filed before the code.** The step is built on `fix/gym-008-follow-up` after this row
is committed, which is the order the `/improve` skill asks for and the order GYM-010 and
GYM-011 did not follow.

---

## Evidence

**The approved GYM-008 proposal promised it.** Its "Where the specs live" says a spec
"rots visibly when the code moves — an anchor that stops matching is requirement 8
firing in CI rather than a stale file nobody opens." Nothing in CI reads a spec:
`.github/workflows/ci.yml` at `c0e1534` names neither `test/mutation/` nor
`scripts/mutate.cjs`. An anchor that stops matching is found only when an author next
runs that round, which for a package's spec may be never.

**The review of #71 found the gap** (DECISIONS-LOG 2026-10-11, the entry recording the
merge), and the Architect chose a `ci.yml` step over a vitest case, a later package, or
correcting the proposal's wording (DECISIONS-LOG 2026-10-11, GYM-008 follow-up).

**Specs are about to multiply.** M9-PLAN §6 says every M9 package owes a round from a
checked-in `test/mutation/<package>.json`. Each one is a list of exact strings in
production code, and each refactor of that code can strand them.

## Proposal

One step in `ci.yml`'s `code` job, directly after `Install`:

```yaml
- name: Mutation specs still match their code (GYM-012)
  if: steps.scaffold.outputs.present == 'true'
  run: |
    shopt -s nullglob
    specs=(test/mutation/*.json)
    if [ ${#specs[@]} -eq 0 ]; then echo "no spec under test/mutation/: GYM-012 checks nothing"; exit 1; fi
    for spec in "${specs[@]}"; do node scripts/mutate.cjs "$spec" --check || exit 1; done
```

`--check` runs everything a round does before its baseline and nothing after it
(TEST-STRATEGY §10): the spec's shape, a control and at least one real mutant, every
path, a clean working tree, every round file committed and plainly tracked, and every
anchor matching its file exactly once. It runs no test and writes nothing.

It sits after `Install` because the checkout is clean there: `npm ci` writes only the
ignored `node_modules`. The later steps run tools that may write files, and a
clean-tree check placed after them would have to know what each one writes.

A step with no spec to check fails and says so, rather than passing having checked
nothing.

## Cost & risk

**Effort:** the step above, and its place in TEST-STRATEGY §10. A `--check` starts a
handful of git processes and reads the spec's files: well under a second for each spec.

**Blast radius:** the required `Typecheck · lint · test` check, on every push and PR.

**What could regress:**

- **A refactor must now carry its spec.** A change that rewrites a line a spec anchors
  fails CI until the spec is updated in the same change. That is the point. The failure
  names the mutant and the file: `M1: its anchor matches scripts/mutate.cjs 0 times`.
- **A false failure from an unclean tree.** If a step before it ever writes a tracked
  or unignored file, `--check` refuses with the clean-tree message. Placing it straight
  after `Install` leaves only `npm ci` before it.
- **A spec whose files `--check` refuses for reasons CI does not share.** A hard link or
  a link on a round file's path is a property of a working copy, and a CI checkout has
  neither. None is known.

**What it binds later.** `--check`'s refusals (`specProblems`, `fileProblems`, the
anchor count) now run inside the required check, so a future change to what they refuse
changes a CI gate and takes its own Gymnasium row (Architect, DECISIONS-LOG
2026-10-11). The refusals this change itself adds, such as a spec of controls only, are
covered by this row.

## Success metric

Binary, all three:

1. **At landing,** a planted rot fails the step's exact command: one anchor's line
   reworded in a scratch tree makes the command exit non-zero, naming the mutant and the
   file. It is shown locally with the command the step runs.
2. **Through 2026-10-25,** the step fails no run on `main` or a PR branch whose specs'
   anchors all match. Any such failure is a false positive.
3. **Through 2026-10-25,** every run of the job prints a `CHECKED` line for every spec
   under `test/mutation/`.

If any fails, the row is `regressed` and the step is removed.

## Rollback

Delete the step from `ci.yml`. Nothing else depends on it: the tool, its tests and the
specs are unchanged by its removal.

## Related

- [GYM-008](./GYM-008-a-mutation-round-is-a-tool-not-a-scratch-script.md): the tool
  and the sentence this row makes true
- [TEST-STRATEGY §10](../../TEST-STRATEGY.md): what `--check` refuses
- `docs/DECISIONS-LOG.md`: 2026-09-02 (a changed CI gate gets a ledger entry), and the
  2026-10-11 GYM-008 follow-up entries
- [ADR-0015](../../adr/ADR-0015-gymnasium-self-improvement.md): the loop and authority
  table this row is filed under; [ENGINEERING-STANDARDS](../../ENGINEERING-STANDARDS.md)
  §3

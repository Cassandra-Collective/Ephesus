# R1.0 — The design of the Rust home and every persisted format

**Date:** 2026-10-19 to 2026-10-20 · **Branch:** `feature/r1-0-home-design` (stacked on
`feature/r0-3-standards-learn-rust`) · **Package:** R1.0 (`docs/RUST-PLAN.md` §5) ·
**Decisions:** DECISIONS-LOG 2026-10-19 (R1's package list, every format redesigned
from first principles) and 2026-10-20 (the design approved, the cursor removed,
SDD-RUST as the Rust tree's SDD)

## 1. Problem / motivation

ADR-0041 §3 says every format the clean break redesigns "is specified in the new SDD
before it is built". So R1 could not start with code. The Architect then chose to
redesign every format from first principles, with today's used only as a checklist of
what must be covered. No checklist existed: SDD §2 and §4 had drifted from the code
(the inventory found 14 persisted artifacts missing from SDD §2 alone).

## 2. What changed

| File | Change |
|---|---|
| `docs/sdd/SDD-RUST.md` | New. Its scope and precedence rule, nine design principles, §2 (the home's layout) and §4 (every persisted format: eight rules, a row per artifact, the problems closed, and §4.13's contract amendments). Approved 2026-10-20 |
| `docs/DECISIONS-LOG.md` | The R1 and R1.0 decisions |
| `docs/RUST-PLAN.md` | R1.0 inserted before R1.1. Coverage floors start in R1.1 |
| `docs/PROGRESS.md` | R1's packages listed; R1.0 ticked; the resume block points at R1.1 |
| `README.md` | The landed marker moves to R1 (`R1.0`), with R0 kept as the previous phase's story. `check-readme-current.cjs` required this the moment R1.0 was ticked |

No code changed.

## 3. Implementation approach

**Inventory first, from the code, not the SDD.** A read-only agent surveyed every
writer and reader in the TypeScript tree and reported each persisted file with:

- its format and version;
- its writers and readers, other processes included;
- every field;
- its invariants, and what absent and damaged mean.

It also listed the enum vocabularies, the facts stored twice, and thirteen format
problems.

**The cross-check before use** (the advisor's, from memory's *absence in one vocabulary
is not absence*): each of the 36 `schemaVersion` modules in `src/shared/` was matched
against the report. 31 were cited by file. `gh-token` and `recall` were covered under
the sockets by their routes, not their module names. `characters`, `emotes` and
`tileset` turned out to be the renderer's bundled art-pack manifests, not home files,
and became a row deferred to R13.

**A row per artifact:** *today*, *designed*, *departs because*, and *absent and damaged*.
"From first principles" was read as "every field carries a stated reason", not as
"every field changes".

**A review changed the draft before the Architect saw it.** The first draft had renamed
paths and fields the contract names: `registry.json` to `roster.json`, `log.jsonl` to
`log/`, mail out of `agents/<id>/`, and the message's fields to camelCase. It had also
split `config.json`. The split contradicted ADR-0032, which decided consent lives in
`config.json` and refused a separate file. A rename for taste costs a line of the
contract, so the revision keeps every contract-named path and field unless a
mechanism demands a change. The P4 defect, `config.json` overwritten when damaged, is
fixed by the store's refuse-on-damaged rule instead of a split. Each remaining contract
change is listed in §4.13, in the *was → proposed* form RUST-PLAN §10 used.

## 4. Mathematical / statistical details

Not applicable: a design document.

## 5. Design decisions

- **The eight rules of §4.0 instead of per-file fixes.** Most of the thirteen problems
  are one rule broken in several places: an unvalidated write, a missing version, a
  stored copy of live state. A rule fixes the class, and a row records only the
  exceptions.
- **The cursor went to the Architect, not into a row.** FR-3.6, NFR-6, ADR-0003 and
  ADR-0013 name it. Removing it was the better design (it had no reader), but it was
  not the engineer's to decide.
- **Sockets and the secret backend were left to their phases** (R3, R7, R6), so this
  approval covers only persisted formats.

## 6. Verification

- The inventory's coverage: 36 of 36 schema modules accounted for (31 cited, 2 by
  route, 3 deferred as bundled assets). Tier A.
- `node scripts/check-readme-current.cjs`: `README landed list is current for R1 (1
  package(s)).` The check was red first, naming R1.0 as undescribed and R0's names as
  no longer current, until the README was updated. Tier A.
- Approval: the Architect's answers of 2026-10-20 (DECISIONS-LOG). Tier C.

**Not proven:** that the design builds. R1.1 to R1.4 prove it, and `armed`'s removal
waits on R1.4's verification that arming is all-or-nothing.

## 7. Related docs

`docs/sdd/SDD-RUST.md` · ADR-0041 · ADR-0003, ADR-0013, ADR-0018, ADR-0032 ·
`docs/RUST-PLAN.md` R1 · SRS FR-3.1, FR-3.6, NFR-6

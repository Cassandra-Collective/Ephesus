# Stoa watchlist — fixture with defective rows

A fixture, not the repository's real watchlist. Every row below except the first is
a way a human-maintained table has held, or could hold, a cell the product must not
take at its word (DECISIONS-LOG 2026-10-11). The seed must leave out and report
the rows the schema refuses, and seed the rest with intake refused.

| ID | Source | Tags | License | Pin | Notes (what to learn) |
|---|---|---|---|---|---|
| src-fixture-good | https://example.invalid/fixture/good | `agent-loop` | MIT | `a1b2c3d` | The control row: seeds, studiable, intake permitted. |
| src-fixture-long | https://example.invalid/fixture/long | `agent-loop`, `tool-use` | unverified (read as AGPL-3.0 on 2026-10-02; **pattern-learning only whatever verification finds** — AGPL code intake is refused under FR-13.5 for this MIT repository) | *(set at first study)* | The 166-character License cell `src-odysseus` carried on main at 75ec37c: over the schema's 120, so it must not be seeded. |
| src-fixture-unverified | https://example.invalid/fixture/unverified | `memory` | unverified | `c3d4e5f` | The honest "not checked": seeds, and intake is refused with the note that says nobody has checked. |
| src-fixture-annotated | https://example.invalid/fixture/annotated | `memory` | unverified (read as MIT) | `d4e5f6a` | A reading kept in the License cell, short enough to pass the schema: seeds, and intake is refused. |
| src-fixture-pending | https://example.invalid/fixture/pending | `tui` | Unverified — pending | `e5f6a7b` | Another spelling of "not checked": seeds, and intake is refused. |
| src-fixture-copyleft | https://example.invalid/fixture/copyleft | `evaluation` | AGPL-3.0 | `f6a7b8c` | A verified license intake may not come from: seeds, and intake is refused. |
| src-fixture-nourl | *(url to follow)* | `orchestration` | MIT | `a7b8c9d` | A source row with no url: must be reported, not skipped in silence. |

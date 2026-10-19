# Ephesus — Software Design Description for the Rust build (SDD-RUST)

**Status:** PROPOSED by R1.0 (2026-10-19), for the Architect's approval. Nothing here is
built until it is approved.

**Scope and precedence, proposed with this file.** This document governs the Rust
workspace (`crates/`, `xtask/`). `docs/sdd/SDD.md` keeps governing the frozen
TypeScript tree until R14 deletes it (ADR-0041). Within its scope the contract's order
holds unchanged: **SDD-RUST > ADR > SRS > README** for *how*, and **SRS > SDD-RUST**
for *what*. A section grows phase by phase. R1.0 writes §2 (the harness home) and §4
(every persisted format). The other sections are named so the numbering matches the
TypeScript SDD's, and they are written by the phases that build them.

| § | Section | Written by |
|---|---|---|
| 1 | Process architecture | ADR-0041 states it; written out at R7 (the headless core) |
| 2 | The harness home: on-disk layout | **R1.0** |
| 3 | The engine adapter surface | R2 |
| 4 | Persisted formats: every schema, and what absent and damaged mean | **R1.0** |
| 5 | The command and event surface (`eph-api`) | R4 |
| 6 | The avatar state machine | R3 |
| 7 | Key sequences | the phase that builds each |
| 8 | The Herald | R12 |
| 9 | The Watch's enforcement points | R6 |
| 10 | Error handling and recovery invariants | R1 (the store), extended by each phase |
| 11 | Performance budgets (NFR-1 to NFR-4, GYM-013) | R4 onward |
| 12 | Traceability | every phase |

---

## R1.0 — design principles for the home and its formats

The Architect's instruction (DECISIONS-LOG 2026-10-19): **every format is redesigned
from first principles, with today's used only as a checklist of what must be covered.**
"From first principles" means every field carries a stated reason. It does not mean
every field changes. Where the honest reason is that today's shape is already the right
one, the row says so.

These principles bind every row in §2 and §4. Each comes from a decision that stands or
a defect this project already paid for.

**P1. Owned formats are redesigned; external ones are read as they are.** Ephesus owns
the home's app-local records, the Agora's files, the Odeon, Gymnasium and Stoa stores,
and the protocols of its own sockets. It does not own the engine's settings file,
transcript, trust record and config directory, MemPalace's store, git's own files, or
`gh`'s output (ADR-0041 §4, RUST-PLAN §2). Each of those gets one row reading
*external, read as-is*, its ground truth cited, and nothing else.

**P2. Whatever an agent reads or writes is text a person and a model can read.** Mail,
memory, identity, the protocol and the board are markdown or JSON, never binary or
tabular (ADR-0003, ADR-0006). An agent is a language model working through a file tool,
and a format it cannot read is a format it will corrupt.

**P3. The Agora is a git repository with one committer** (ADR-0004). Its files are
diffable, written by one process, and never merged. State that changes faster than a
commit, or that no agent needs, lives in the app-local home outside the Agora.

**P4. Every schema'd file carries its version, and a reader refuses a version it does not
know** (invariant 9). A reader accepts an older version only through a named migration
on read. The memory *an enum rename is a schema migration* is why: a value cached in a
durable file outlives the code that wrote it.

**P5. Every file defines what ABSENT means and what DAMAGED means, separately.** Absent is
an ordinary state (a first run, a feature never used) with a stated default. Damaged
(unparseable, an unknown version, a failed validation) is a visible degradation naming
its consequence, never read as absent. Memory records three defects of this one shape:
*seeded only when absent*, *absent ≠ damaged*, and *a catch guards nothing if the callee
never throws*.

**P6. A file another process reads is written atomically** (temp file, flush, rename;
invariant 3). **An append-only file is opened for append only, and its reader tolerates
a torn last line** (invariant 5). In Rust these are the only two write paths, and
`eph-fs` owns both (ADR-0041's table, row 3).

**P7. One fact, one place.** A fact stored twice is two fields that can disagree, which
memory records as an equivalent mutant pointing at a design smell. Derived state is
re-derived on read and given no store (ADR-0027 §5).

**P8. Every loss is reported with its consequence** (invariant 7). A record that cannot be
restored becomes a degradation that says what the Architect loses, not a silent default.

**P9. Formats carry no secrets** (invariant 6), and the book of record carries no secret
value. A credential reaches an agent only as an environment variable at spawn.

### The row each artifact gets

| Column | Says |
|---|---|
| **Today** | The TypeScript format, from the inventory, with its validator cited (`src/shared/…`) |
| **Designed** | The Rust format: path, encoding, version, fields |
| **Departs because** | One reason per departure, or "unchanged: today's is the right shape, because …" |
| **Absent / Damaged** | What each means on read, and what the harness does |
| **Writer / Readers** | The one writer, and every reader (other processes included) |
| **Owned** | Owned, or external (P1) |

---

## 2. The harness home: on-disk layout

*Written by R1.0 from the inventory. Pending.*

## 4. Persisted formats

*Written by R1.0 from the inventory. Pending.*

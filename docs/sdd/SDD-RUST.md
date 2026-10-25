# Ephesus — Software Design Description for the Rust build (SDD-RUST)

**Status:** §2 and §4 APPROVED by the Architect on 2026-10-20 (R1.0; DECISIONS-LOG), and
§10 on 2026-10-25 (R1.4a). R1.4b's edits to §4.5 and §10 apply the Architect's answers
of 2026-10-25 (DECISIONS-LOG, *R1.4b — Architect decisions*, three rounds). The other
sections are written, and approved, by the phases that build them.

**Scope and precedence (approved 2026-10-20).** This document governs the Rust
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

The checklist is the 2026-10-19 inventory of every file the TypeScript build persists:
about 45 files and directories across 13 areas. The 36 `schemaVersion` modules of
`src/shared/` each map to a row of it, or to the three bundled art-pack manifests (R13).

**How first principles met the contract.** The redesign kept a path or a field name
wherever the only reason to change it was taste. The SRS, ADR-0003, ADR-0018, ADR-0032
and invariant 4 name many of these paths and fields (`registry.json`, `log.jsonl`,
`gate-policy.json`, `config.json`, `agents/<id>/inbox/`, the message's fields). Renaming
one buys a nicer name and costs a line of the contract the Architect approved. **Every
change below is driven by a mechanism**: a defect the inventory found, or a rule a
format did not meet. Each change that touches contract text is listed, *was →
proposed*, in §4.13.

```
<home>/                       ~/.ephesus, or $EPH_HOME (unchanged)
  home.json                   NEW: marks a home this build made (§4.4); written first
  config.json                 the Architect's settings and the company's mode and consent (§4.1)
  gate-policy.json            the gate policy (FR-11.1, FR-1.9, FR-11.7)
  authority.json              Artemis's delegated authority (FR-5.5)
  github-app.json             the company identity (ADR-0022)
  known-targets.json          targets offered for activation (a convenience)
  activations.json  gates.json  drafts.json  triggers.json  breaker-stops.json
                              the restart records, all through one store (§4.2)
  secrets.enc                 the broker's store; its backend is R6's (§4.3)
  control-endpoint.json       where control.sock is, written at bind, removed at quit
  events.sock | pipe          the agents' endpoint (socket protocol: R3)
  control.sock | pipe         the Architect's front door (ADR-0033; protocol: R7)
  db.sqlite                   the durable cost ledger and app-local tables (§4.10)
  index/                      DERIVED: fts.sqlite (rebuilt when damaged) and MemPalace's store
  usage/<agentId>.json        engine usage reports, written by eph-usage
  prompts/                    OVERRIDES ONLY: a file here replaces the bundled one (§4.4)
  profiles/  instances/  tools/  scratch/
  engines/<engine>/<agent>/   each hire's private engine install (external, P1)
  worktrees/<agentId>/        git worktrees of the agent's target (external, P1)
  DIAGNOSIS.md

<home>/agora/                 git, one committer (ADR-0004)
  .git/ephesus-committer.lock NEW: the committer's OS lock, held while it runs (§4.5)
  .git/EPHESUS_COMMIT_MSG     NEW: the batch message each commit reads with -F (§4.5)
  .gitignore                  NEW: rendered at every boot; keeps .inflight/ and eph-fs temporaries out (§4.5)
  PROTOCOL.md                 rendered from the bundled copy at every boot (§4.6)
  registry.json               who is hired (fields redesigned, §4.5)
  tasks.json  board.md  knowledge/
  tasks-archive.jsonl         NEW: done tasks the ledger's retention moved out (§4.5)
  log.jsonl  log-archive/     the book of record (lines redesigned, §4.5)
  human/inbox/  human/inbox/.done/
  agents/<id>/                identity.md  memory.md  memory-archive/
                              inbox/  inbox/.inflight/  inbox/.done/  inbox/.rejected/
                              outbox/  outbox/.rejected/
  odeon/  gymnasium/  stoa/
```

**What changed in the layout:**

| Change | Because |
|---|---|
| `prompts/` holds overrides only. Nothing is copied | Inventory P6: a prompt copied on first read never receives a shipped improvement. Memory: *seeded only when absent means design the absence* |
| `PROTOCOL.md` is rendered at every boot | Same defect. A protocol change today never reaches a company that has already booted once |
| `index/fts.sqlite` is rebuilt when damaged | SDD §10 promised it and the TypeScript build never did it (P9). Derived state needs no rescue (ADR-0027 §5) |
| The Agora's `.gitignore` excludes `inbox/.inflight/` | A message in flight is a transient state the book of record already logs. Committing every move doubled commit traffic with no audit value. `.done/` and `.rejected/` stay committed: they are the record |
| The Agora's `.gitignore` also excludes `write_atomic`'s temporaries (R1.3 review, 2026-10-20; narrowed in R1.4a, Architect, 2026-10-25) | `write_atomic` writes `.<name>.<8 hex>.tmp` beside its target, and a write killed before its rename leaves one (R1.2's residual). `add -A` would commit that residue, and a concurrent `add -A` could stage a half-written one. The pattern names exactly that shape, so an agent's own `.notes.tmp` is still committed, and the file is rendered at every boot, before the reconcile (§4.5) |
| `home.json` marks a Rust home (Architect, 2026-10-20) | The TypeScript build keeps running from `~/.ephesus` until R14, and six of its files are formats this build cannot read. An existing directory without the marker is never written to: a TypeScript-era home is reported once, with the fix (`EPH_HOME`), and left exactly as it is. Only a path that does not exist, or an empty directory, becomes a new home, and the marker is its first file |
| `.git/ephesus-committer.lock` holds the committer's lock (Architect, 2026-10-20) | Clearing a stale `index.lock` is safe only if nothing else commits to the Agora, and one harness per home (ADR-0034's probe) arrives in R7. The committer holds an OS lock on this file for its whole life, so a second committer is refused, and the OS releases it when the process dies, so it never goes stale. It holds no bytes and is never deleted. It sits in `.git` so no `add -A` sees it |
| `cursor.json` removed (Architect, 2026-10-20) | It had no reader (P7). Idempotency comes from the state directories, as it already did. FR-3.6 and NFR-6 are amended and ADR-0003 and ADR-0013 get clause notes (§4.13) |

Everything else in the tree keeps today's name and place.

## 4. Persisted formats

### 4.0 Rules every format follows

These turn P4 to P9 into mechanics. Each row below states only where a file departs
from them.

1. **One version field.** `"schemaVersion": <int>` goes in every JSON document and on
   every JSONL line, and `PRAGMA user_version` in every SQLite file. A file has exactly
   one: the versions nested inside records are removed (inventory §11). An unknown
   version reads as DAMAGED. An older one is read only through a named migration (P4;
   memory: *an enum rename is a schema migration*). **A format whose shape changed from
   today's starts at version 2** (Architect, 2026-10-20), so a TypeScript-era file at the
   unchanged home path reads as older, not as damage of an unknown kind. A format whose
   shape did not change keeps today's version 1, because today's file is that format.
   R1.2 marks the home itself, so a TypeScript-era home is reported once.
2. **One time format.** Every new or redesigned timestamp is RFC 3339 in UTC with
   milliseconds (`2026-10-19T12:34:56.789Z`). The book of record's `ts` moves from epoch
   milliseconds to this form (§4.5). A filename that carries a time uses the same
   instant in compact form (`20261019T123456789Z`). Today three time forms coexist.
3. **One id grammar for the ids the harness mints:** `<kind>-<compact UTC>-<8 hex>`,
   with kind `msg`, `t`, `g`, `m`, `mt`, `brief` or `act`. The short kinds are today's
   prefixes, so `task.id` keeps its `t-` shape. The ids sort by time and are
   filename-safe. Today there are five grammars (inventory §3.7, §5).
4. **One reference grammar.** An artifact reference is a path relative to the Agora's
   root (`odeon/decks/…`). A log entry is `log:<seq>` and a message is `msg:<id>`. Today
   decks are Agora-relative and briefs home-relative (P8), and `task.source.log` is
   documented as one thing and written as another (P7).
5. **Two write paths.** `eph_fs::write_atomic` (temp, flush, fsync, rename, with the
   Windows rename retry) serves anything another process reads. `eph_fs::AppendLog`
   serves the append-only files (P6), and `eph_fs::append::append_line` the one whose
   lines carry no `seq` (the task archive, §4.5).
6. **One store for structured records.** `eph_agora::Store<T>` validates on read and
   **before every write**, distinguishes ABSENT from DAMAGED, and takes a per-record
   **on-damaged policy**:
   - **Refuse**: the record reads as damaged, and every write is refused until the
     Architect repairs it, with the evidence left on disk.
   - **Block**: as Refuse, and the dependent action is refused too.
   - **Rebuild**: the file is derived; it is deleted and rebuilt.

   No record is ever written over a damaged file. Today `registry.json` and
   `tasks.json` are written without validation (P3), and `config.json` and the Stoa
   watchlist are written over when damaged (P4, P5).
7. **Live process state is not stored** (ADR-0027 §5, P7). A field that mirrors what a
   running process or an engine is doing is derived at runtime and shown, never
   persisted.
8. **Seeded files record their origin.** A file written once from a shipped default
   (`gate-policy.json`, `authority.json`) carries `"seededFrom": <shipped version>`. At
   boot the harness compares that with the shipped version and reports drift
   (`home/policy-behind:<file>`). It never overwrites the Architect's edits, but a
   shipped improvement is now at least disclosed (P6).

### 4.1 The home's settings and policy

| File | Today | Designed | Departs because | Absent / Damaged |
|---|---|---|---|---|
| `config.json` | `{mempalaceCommand?, mode?, consent?, everEnabledImproving?, wakeWordEnabled?, pacing?}`. Two writers: the Architect and the harness (mode change, consent grant) | Unchanged fields and one file. ADR-0032 decided consent lives here and refused a file of its own; ADR-0018 holds the mode here. It is written only through the store with **Refuse**. Durations in `pacing` become duration strings (`"90s"`). The Herald's model and voice ids move in from `prompts/herald/*.md` (`voice{modelId, voiceId}`) | Inventory P4: `saveConfig` wrote defaults plus a patch over a damaged file. The store fixes that without splitting the file. The Herald's ids are configuration, not prose (P12) | Absent: defaults, seeded `{schemaVersion}`. Damaged: run on defaults, report it, and refuse every write. A mode change or consent grant asks for the repair first |
| `gate-policy.json` | `autonomy`, `maxDailyTokens?`, `rules[{kind, autonomy, maxSpendTokens?, channels?, requireRepeatBack?}]` | Unchanged fields, plus `seededFrom` (rule 8). `rules` becomes a map keyed by gate kind | "At most one rule per kind" holds by construction instead of a refine check. The fields are FR-11.1, FR-1.9 and FR-11.7's | Absent: deny-all, reported (FR-11.1 can only loosen). Damaged: deny-all; the FR-11.7 ceiling write refuses (unchanged) |
| `authority.json` | `grants[{class, domains, maxSpendTokens?}]` | Unchanged, plus `seededFrom` | Absent is now reported, matching `gate-policy.json` (P6: one was silent, the other warned) | Absent: no delegated authority, reported once. Damaged: no authority, Refuse |
| `github-app.json` | `{appId, installationId?, slug?}` | Unchanged | Its shape is ADR-0022's | Absent: no company identity, silently (a feature not configured). Damaged: reported |
| `known-targets.json` | `targets[{profile, target{kind, id, path}, lastUsedAt}]`, trimmed to 50 | Unchanged, through the store with **Rebuild** | A convenience. Losing it costs a retyped path, so damage is reported and the next activation rewrites it, as today | — |

### 4.2 The restart records (ADR-0027), all through the one store

| File | Designed | Departs because | On damaged |
|---|---|---|---|
| `activations.json` | Today's shape: `plan` verbatim, `crew`, `pendingEvents`, ids and times per §4.0. **`armed` is derivable and is removed** by the phase that types this file. Verified in R1.4 (2026-10-20): it is every trigger of the plan that has an interval (`everyMs`), while the crew is live, and none while the crew is down (`src/main/profiles.ts:602–615, 848–851`). The question as R1.0 posed it, "always every trigger", is false, and the rule found is a function of `plan` and `crew` all the same | `armed` is a stored copy of a fact derivable from `plan` and `crew` (P7). It would stay only if some path armed a set the plan and the crew do not determine, and none does | Refuse: the company comes back un-hired, reported with that consequence |
| `gates.json` | `open[{id, kind, agentId, because, channel, packaging{what, why, blastRadius, rollback}, taskId\|null, requiresRepeatBack, memoTrigger\|null, openedAt}]`, `settled[{id, verdict, settledAt}] ≤1000` | The nested `schemaVersion` in each gate goes (rule 1). `settledAt` lets the trim keep the newest by time rather than by id order | Refuse: the approvals queue is empty and each blocked task is reported as orphaned |
| `drafts.json` | Today's shape without the nested version, as a map keyed by gate id | "No two drafts on one gate" holds by construction | Refuse: a restored gate whose draft did not come back is reported (ADR-0030) |
| `triggers.json` | `{lastFired: {id → time}}` | Rule 2 | Refuse: every trigger is due, reported |
| `breaker-stops.json` | Today's shape, keyed by agent | The one store now carries the safety contract that made this file a special case, through its policy | **Block**: every start and respawn is refused, as today |

### 4.3 Secrets and the endpoint file

| Artifact | Designed | Departs because |
|---|---|---|
| `secrets.enc` | Name and location kept until R6 chooses a backend with its crate memo. The OS keychain is the candidate, with this file becoming an index of names and rotation dates and never a value | Electron's `safeStorage` does not exist in Rust, so the backend must change, and R6 owns that decision. Today the file is plaintext JSON around base64 ciphertext (P12) |
| `control-endpoint.json` | `{schemaVersion, endpoint, pid}` | P7: `startedAt` has no reader. It stays a courtesy, not a lock, because occupancy is probed (ADR-0034) |
| The two sockets | Paths and Windows pipe names unchanged. *Proposed for R3 and R7, not decided here:* newline-framed JSON instead of HTTP, because Ephesus now owns both ends and HTTP would need a crate outside Set A | — |

### 4.4 Other home files

| Artifact | Designed | Departs because |
|---|---|---|
| `usage/<agentId>.json` | Today's fields with times per rule 2. Written by the Rust `eph-usage` through `write_atomic` | The writer gains the Windows rename retry today's `.mjs` copy lacks (P10) |
| `prompts/` | Overrides only. The bundled prompt is used unless `prompts/<same path>` exists | P6. An unreadable override is reported, and the bundled prompt is used |
| `profiles/<name>/triggers/*.json` | Gain `schemaVersion` | The one bundle file without one (P1) |
| `home.json` | NEW: `{schemaVersion, createdAt}`, written once, when this build creates the home, before anything else. Absent from an existing non-empty directory: the directory is not this build's, and nothing is written to it. Damaged: the same, reported with the repair | The TypeScript build shares the path (§2) |
| `instances/`, `tools/`, `scratch/`, `DIAGNOSIS.md` | Unchanged | Frozen copies, Architect-owned directories, read-only checkouts, and a human-facing render: nothing about them was wrong |

### 4.5 The Agora: registry, ledger, board, book of record, mail

| Artifact | Designed | Departs because | Absent / Damaged |
|---|---|---|---|
| `registry.json` | `{schemaVersion, orchestratorId\|null, agents: {id → {name, role, engine, capabilities, seat, envGrants, profile\|null, cwd, target\|null, budget\|null, hire{template, version}, spawnedAt, retiredAt\|null}}}` | Rule 7: `status`, `hookFidelity` and `lastSeen` describe a running process or an engine, so they are derived and shown, not stored. `retiredAt` is the one lifecycle fact routing needs, since mail to a retired agent bounces. `isOrchestrator` copied `orchestratorId` and goes (P7). `target` held a 4096-character cwd against a 256-character schema (P3), so `cwd` and `target` are now separate. `hire` is now written (P7) | Absent: an empty roster. Damaged: Refuse |
| `tasks.json` | Today's fields; ids, times and refs per §4.0 (`source: {kind, ref}`). Validated as a whole ledger (Architect, 2026-10-20): unique ids, every dependency in the ledger and not the task itself, and no `done` task missing a required review artifact or blocked by a gate | Rules 3 and 4. **Retention (Architect, 2026-10-20):** once the ledger holds more than 9,000 tasks (of its 10,000 cap), the done tasks beyond the newest 1,000 done (by `updatedAt`) move to `tasks-archive.jsonl`, except any task a kept task depends on, transitively, because a dependency outside the ledger makes the ledger damaged. The archive is append-only, one `{schemaVersion, archivedAt, tasks}` line per retention run, because each line is one synced append and one line per task took 62 s for 8,001 tasks (R1.4a; Architect, 2026-10-25). The archive and the Agora's history keep every task | Refuse |
| `tasks-archive.jsonl` | NEW (R1.4a): append-only through `eph_fs::append::append_line`, which reads only the file's last byte, so a write costs the same however large the archive grows (Architect, 2026-10-25). One `{schemaVersion, archivedAt, tasks}` line per retention run, holding 1 to 10,000 done tasks with unique ids, each held to the ledger's own per-task rules. `schemaVersion` is the ledger's, because a line holds the ledger's task, so a change to a task's shape is a new version of both. Writer: `Agora::write_tasks`, which appends the line before it writes the trimmed ledger. Reader (R1.4b, Architect, 2026-10-25): `Agora::archived`, which returns every archived task the ledger does not hold, the newest line winning a repeat, and lets the ledger win, because a write cut short between the two files leaves a task in both. It counts lines as the book's reader does: a torn last line skipped, a newer build's line unknown, any other bad line damaged. A damaged ledger refuses the read, since which tasks it holds is unknown | The retention moves done tasks out of the ledger without losing them (Architect, 2026-10-20) | Absent: nothing has been archived. Damaged: an archive the writer cannot append to refuses every ledger write that would archive, and the ledger is left as it was. A torn last line is left on disk, as in every append-only file |
| `.gitignore` | NEW (R1.3, R1.4a; Architect, 2026-10-25): owned by the harness. Rendered at every boot from the bundled text, before the reconcile, so crash residue never reaches history: `agents/*/inbox/.inflight/` and `write_atomic`'s temporary name, `.<name>.<8 hex>.tmp`, and nothing wider. A directory of that temporary name is re-included (`!<pattern>/`, R1.4b, Architect, 2026-10-25), since the pattern alone hid a directory and everything in it, and `write_atomic` never makes one. No home override. It is replaced unless it is a regular file holding exactly those bytes, because git does not follow a `.gitignore` that is a link. Writer: the boot. Reader: git. **What any other ignore file hides is named** (R1.4b): each batch's post-stage status lists what git ignores, in the walk it already makes, and every entry that is not one of the two shapes the harness makes (`agents/<id>/inbox/.inflight/`, or a FILE named `.<name>.<8 lowercase hex>.tmp`) is named in `agora/hidden-work`, cleared once nothing is; the boot asks once more when no batch landed | Rendered rather than seeded, an edit or a link cannot widen or empty what history keeps out. git honours an ignore file anywhere in the tree, and the hermetic configuration cannot turn that off, so an agent's own `.gitignore` is detected rather than prevented (Architect, 2026-10-25) | Absent or different: rendered. A render that fails is a condition (`agora/render:.gitignore`), and the boot goes on. A listing git cannot make is `agora/ignore-check` |
| Attributes | **None are read from the tree** (R1.4b, Architect, 2026-10-25): every Agora git call passes `--attr-source` set to the empty tree, so an agent's `.gitattributes` is committed as a file and changes nothing. Needs git 2.40 or newer; an older git's refusal names the fix | Measured on git 2.53: one agent's `* working-tree-encoding=UTF-16` failed every `add -A` on the Agora, and `text` rewrote line endings in history. No attribute hides a file, so prevention costs nothing a detection would keep | A SHA-256 Agora refuses the SHA-1 empty tree on every call, loudly; the Agora is made SHA-1 (R1.3) |
| `board.md`, `knowledge/` | Unchanged. The board is seeded once, when absent, from the home's override (`prompts/agora/board.md`, §4.4) or else the bundled text, and is written whole | Agent-facing text with one writer (P2, ADR-0005) | Absent: reads as empty, which is not a degraded state |
| `log.jsonl`, `log-archive/` | Every line is `{schemaVersion, seq, ts, kind, event, source?, refs?, data}`, with `ts` in rule 2's form. `kind` and `event` come from a closed vocabulary per kind, and the writer cannot emit an unknown pair. The reader keeps a line it does not know as an opaque, counted record. Rotation at 4 MiB is unchanged. **R1.4b (Architect, 2026-10-25):** one writer, `eph_agora::book::Book`, takes one lock per append, stamps `seq` and `ts`, syncs the line, then publishes it to every subscriber's own channel in file order, then returns. A reader's cursor is one byte offset into the sealed segments, oldest first, and the live file, read as one stream, which a rotation does not move, because rotation is a rename. The reads are `since(cursor)` and `tail(n)`, and each returns the records it knows, its counts of unknown and damaged lines, whether the live file ended mid-line, and the cursor after its last whole line | Today 27 kinds carry free-string events with open fields (inventory §10, P1), so no reader could know what a line may say. A typed writer and a tolerant reader let the format grow without one release breaking an older log. **The writer assumes one harness per home** (ADR-0034 is a probe, not a lock), so a reader's cursor is a position, never a `seq`, which a second harness on one home repeats. *Correction (R1.4b):* this row first said readers "keep today's position-based cursor". Today's reader pages by `seq` (`src/main/eventlog.ts`), so the position-based cursor is this design's, not today's | Absent: a new log. A torn last line is ignored and left on disk (unchanged): a read skips it without consuming it, so the line reads whole once finished. A sealed segment's unterminated line, and any other line that is not a record, is damage, counted. A live file that ends mid-line at boot is `agora/log-torn`: the last run was killed mid-append |
| `agents/<id>/inbox/`, `.inflight/`, `.done/`, `.rejected/`, `outbox/`, `outbox/.rejected/`, `human/inbox/` | **Unchanged layout:** a rename between state directories is the atomic state machine, and invariant 4 and ADR-0003 give an agent exactly its own directory. `.inflight/` is no longer committed | Keeping the layout keeps one directory per agent as the whole of what its engine may write: one grant | An unparseable incoming file goes to `.rejected/` with a teaching reply (unchanged) |
| `.git/ephesus-committer.lock` | NEW (R1.3; Architect, 2026-10-20): an empty file the committer holds an OS lock on (`File::try_lock`) from open until its last git has finished. Writer: the committer, through `eph_fs::lock`, which opens it in place, never truncates it, never deletes it, and on Windows denies delete sharing. Readers: none; the OS lock is the whole format, and any `Worktrees::create` treats a git directory holding it as the Agora | Clearing a stale `index.lock` is safe only if nothing else commits. ADR-0034's probe owns one harness per home in R7, and this owns one committer per Agora now, with no stale-lock policy because the OS releases it when the process dies | Absent: created at open. Damaged: it has no content to damage; a lock held elsewhere is a refusal (`OpenError::Held`) that names the fix |
| `.git/EPHESUS_COMMIT_MSG` | NEW (R1.3): the batch message, written with `write_atomic` before each `git commit -F`. Writer: the committer. Reader: git | A message on the command line is capped by the OS (about 32,000 characters on Windows), so a large batch could never land | Absent: written before the commit that reads it. Damaged: not reachable, since it is rewritten whole before every read |
| `agents/<id>/cursor.json` | **Removed** (Architect, 2026-10-20). Idempotency comes from the state directories: a message's file is in exactly one of `inbox/`, `.inflight/` and `.done/`, and consuming it moves it by rename | It had no reader (P7) | — |
| The message | `{schemaVersion, id, conversation, in_reply_to, from, to, act, subject, body, hops, needs_human, created_at}` | `schemaVersion` is added (rule 1; agents write messages, so PROTOCOL.md tells them to write it, and the validator refuses one without it). `requires_reply` is removed: it was corrected on read to a function of `act` (FR-3.3), which made it two fields that cannot disagree (P7). The other names are FR-3.1's, kept | — |

### 4.6 Per-agent files and the agent-facing contract

| Artifact | Designed | Departs because |
|---|---|---|
| `PROTOCOL.md` | Rendered at every boot from the home's override (`prompts/agora/PROTOCOL.md`, §4.4) or else the bundled copy, and replaced unless it is a regular file holding exactly those bytes. An override that exists and cannot be read is reported (`home/prompt-unreadable:agora/PROTOCOL.md`), and the bundled copy is used. Its text is today's until R5 corrects what it teaches about the message (Architect, 2026-10-25) | P6. The Agora's history then records every protocol change an agent could have read |
| `identity.md`, `memory-archive/` | Unchanged | Right as they are |
| `memory.md` | Unchanged format (ADR-0006). **New writing rule:** the harness appends through an append-only open, never a read-modify-write, and condenses only while the agent is held at a Stop boundary or is down | Today the harness's read-modify-write raced the agent's own appends (P10). Two appenders through append-only opens do not lose each other's lines. The one rewrite waits for the agent to be still |

### 4.7 The Odeon, the Gymnasium and the Stoa

| Artifact | Designed | Departs because |
|---|---|---|
| `odeon/**` | Unchanged formats. Ids, filenames and refs per §4.0 | Rules 3 and 4. FR-7 fixes the shapes of these human archives |
| `gymnasium/LEDGER.md`, `proposals/` | **Unchanged shapes, as FR-12.6 requires.** The reader joins each row with its proposal file | Today the parse is lossy: `class` reads `craft`, and `proposedBy` reads empty (inventory §6.1). The proposal file already holds both |
| `stoa/watchlist.json`, `briefs/` | Unchanged shapes. The watchlist goes through the store with Refuse | Today `register` and `retire` wrote over a damaged watchlist (P5). The 2026-10-11 Stoa seed decisions apply at R11 |

### 4.8 Message bodies an endpoint receives

Each payload in inventory §3.8 carries its own `schemaVersion`, and each is specified by
the phase that builds its endpoint (R5 to R11) under §4.0's rules. A payload that does
not validate is refused with a teaching reply.

### 4.9 Not in R1.0's scope

| Format | Phase |
|---|---|
| The hook envelope, recall and gh-token on `events.sock` | R3 |
| The control socket's ops | R7 |
| The secret backend | R6, with its crate memo |
| Window geometry | R4 |
| The floor's art-pack manifests (bundled assets, not home files) | R13 |

### 4.10 SQLite

| Database | Designed | Departs because |
|---|---|---|
| `db.sqlite` | `PRAGMA user_version = 1`, WAL. Tables `cost_ledger` (append-only, FR-11.2), `cost_fold_cursor` and `installed_settings`, with today's columns | Rule 1: neither database has a version today (P1). SDD §4.6's `command_history` and `metrics_rollup` never existed and are not designed in (P9) |
| `index/fts.sqlite` | `PRAGMA user_version`. `doc_state` and `passages` (FTS5) unchanged. **Rebuild** when damaged or of an older version | P9 |

### 4.11 External formats, read as they are (P1)

| Artifact | Owner | Ground truth |
|---|---|---|
| `engines/claude/<agent>/eph-settings.json` | Claude Code's settings schema, written by Ephesus in the engine's format | `test/fixtures/engine-output/`; memory *Claude Code engine ground truth* |
| `engines/claude/<agent>/.claude.json` | The engine, which rewrites it wholesale. Onboarding and trust are merged in (ADR-0021, ADR-0025) | As above |
| `engines/claude/<agent>/projects/<slug>/<session>.jsonl` | The engine's transcript, read for the cost ledger | `test/fixtures/claude-limit/` |
| `worktrees/`, the Agora's `.git/` | git, except the two files Ephesus keeps inside `.git` (`ephesus-committer.lock` and `EPHESUS_COMMIT_MSG`, §4.5), which git never reads as its own | RUST-PLAN §2 |
| `index/` (MemPalace's store) | MemPalace | Its CLI, version-probed (ADR-0016) |

### 4.12 Today's format problems, each closed or carried

| Problem | Closed by |
|---|---|
| P1 missing versions | Rule 1. Human-facing markdown carries none, because no program parses it back, except LEDGER.md, whose shape FR-12.6 fixes |
| P2 loose or shared version constants | One named constant per format, in `eph-schema` |
| P3 roster and tasks written unvalidated | Rule 6 |
| P4 `config.json` overwritten when damaged | Rule 6, with the file kept whole (ADR-0032) |
| P5 the watchlist overwritten when damaged | Rule 6 |
| P6 seeded files never refresh | Prompts as overrides, PROTOCOL.md rendered at boot, rule 8 |
| P7 fields nobody reads | `startedAt`, `requires_reply`, `isOrchestrator`, `status`, `hookFidelity`, `lastSeen` and `cursor.json` removed; `armed` after verification; `hire` written |
| P8 inconsistent refs | Rule 4 |
| P9 doc drift | This section. SDD.md stays the frozen tree's |
| P10 non-atomic writers | Rule 5, and `memory.md`'s new writing rule |
| P11 lossy derived cursors | Decided where each subsystem is built: the standup's `lastSeq` at R9, MemPalace's gate at R8 |
| P12 misleading formats | `secrets.enc` at R6; the Herald's ids moved into `config.json` |
| P13 unbounded growth | Named, not designed: a retention rule for `.done/`, `.rejected/`, `memory-archive/`, `odeon/**` and `cost_ledger` is owed at its phase. Done tasks are decided (R1.4a, §4.5) |

### 4.13 Contract text this design requires amending

The SRS is the *what*, and an accepted ADR is never edited. These are the only lines the
design touches. **The Architect approved all of them on 2026-10-20, the cursor's removal
included.** They are applied as dated amendments, and as ADR clause notes, in R1.1's
first commit.

| Where | Was | Proposed |
|---|---|---|
| SRS FR-3.1 | "Messages SHALL be single JSON files using the speech-act schema (SDD §5.3): `id, conversation, in_reply_to, from, to, act, subject, body, hops, requires_reply, needs_human, created_at`." | "…the speech-act schema (SDD-RUST §4.5): `schemaVersion, id, conversation, in_reply_to, from, to, act, subject, body, hops, needs_human, created_at`. Whether a message obligates a reply is a function of `act` (FR-3.3), not a field." |
| SRS FR-3.6 | "…and be idempotent via a per-agent cursor." | "…and be idempotent: a message's file is in exactly one of `inbox/`, `inbox/.inflight/` and `inbox/.done/`, and consuming one moves it by rename." |
| SRS NFR-6 | "…idempotent consumption (cursor)…" | "…idempotent consumption (state directories, FR-3.6)…" |
| ADR-0003 line 26, ADR-0013 line 25 | "per-agent cursor"; "via the cursor (ADR-0003)" | A clause note in `docs/adr/README.md`, the existing place for overtaken clauses: the cursor is replaced by the state directories (SDD-RUST §4.5) |
| ADR-0018, ADR-0032, ADR-0040 | `config.json` holds the mode, the consent and the view | **Unchanged**: the design keeps them in `config.json` |

## 10. Error handling and recovery invariants

Written by R1 (the store, the committer and the boot) and approved by the Architect on
2026-10-25, and extended by each phase that adds a recovery path. Each invariant names
the code that holds it and the test that proves it.

| Invariant | Held by | Proved by |
|---|---|---|
| **Absent is not damaged.** A record's file that does not exist is its ordinary empty state. One that cannot be read, is a link to nothing, is not strict JSON (a byte-order mark included), holds a version this build does not read, or fails validation is damaged, and is never read as absent | `eph_agora::store::Store::load` | `absent_present_and_damaged_are_three_answers`, `a_file_that_cannot_be_read_is_damaged_not_absent`, `a_link_to_nothing_is_damaged_not_absent_and_is_never_deleted`, `a_file_saved_with_a_byte_order_mark_is_damaged_and_says_how_to_fix_it` |
| **Nothing is written over damage.** A write validates the value first, then refuses while the file on disk is damaged under Refuse or Block, leaving the evidence. **Block is enforced by the caller:** inside the store it behaves as Refuse, and the policy travels with the `Damage`, so the caller refuses the action that depends on the record too | `Store::save`, `Store::writable` | `nothing_is_written_over_damage_under_refuse_or_block`, `an_invalid_value_is_never_written`, `a_damage_names_its_cause_and_its_consequence` |
| **A derived record is rebuilt, and nothing unknown is deleted.** Under Rebuild, text that is not a valid record is deleted when it is read. A file that cannot be read is left in place, because what it holds is unknown, and the rebuild writes over it. The damage says which (`Damage::deleted`) | `Store::load` | `a_derived_record_is_deleted_reported_and_rebuilt`, `a_derived_record_that_cannot_be_read_is_left_in_place_and_written_over`, `a_derived_record_that_cannot_be_deleted_says_so` |
| **A file another process reads is whole or old, never partial.** Every such write is a synced temporary file renamed over the target, with Windows' passing holds waited out | `eph_fs::atomic::write_atomic` | `eph-fs` atomic tests (R1.2) |
| **An append-only file only grows.** A torn last line is left as it is and the next line starts fresh; rotation is a rename | `eph_fs::append::AppendLog`, `append_line` | `a_torn_last_line_is_left_alone_and_the_next_append_starts_fresh` and the other `eph-fs` append tests (R1.2), `a_line_with_no_seq_is_appended_whole_after_a_torn_tail` (R1.4a) |
| **One committer per Agora.** An OS lock, released by the OS when the process dies, so a crash leaves no stale lock | `eph_git::committer::Committer::open` | `a_second_committer_is_refused_until_the_first_is_gone`, `a_killed_committer_releases_its_lock` (R1.3) |
| **Only the committer's refusal stops a boot.** `Agora::open` fails only when the committer refuses: another one holds the Agora, or `git init` failed. Every later step that goes wrong becomes a condition, and the boot goes on | `Agora::open` | `a_held_agora_is_refused_in_the_committers_words`, `a_file_the_boot_cannot_write_is_reported_and_the_boot_goes_on`, `a_reconcile_that_cannot_commit_is_reported_and_the_boot_goes_on` |
| **The boot keeps crash residue out of history.** Its order is the book's replay (a read), the committer, the `.gitignore` render, the reconcile (skipped for a repository this boot made over an empty directory), then what the boot found so far onto the degradation channel, the `PROTOCOL.md` render, the seeds, then one commit when anything was written, the book included (R1.4b), named for what it holds: `SEED_SUBJECT` or `RECORD_SUBJECT`, each prefixed by the reconcile's when the reconcile failed. No line this boot writes enters the reconcile, which is the last run's | `Agora::open` | `the_ignore_file_is_rendered_before_the_reconcile_so_crash_residue_stays_out`, `when_the_reconcile_fails_a_rendered_ignore_file_is_still_committed`, `a_boot_that_changes_nothing_commits_nothing`, `the_boots_conditions_stay_out_of_the_reconcile_and_land_in_one_commit` |
| **A `git init` a kill left unfinished is finished** (Architect, 2026-10-25). When `.git` has no `HEAD`, open, holding the committer's lock, waits over the retry schedule while any lock file other than its own and `index.lock` sits directly in `.git` (a killed init's `config.lock`), removes those left, runs `git init`, and names them; the boot reports them as `agora/init-lock`. Otherwise the next `git init` refuses the lock, and so would every boot after it | `Committer::open`, `Agora::open` | `a_git_init_a_kill_left_holding_its_locks_is_finished_and_named`, `open_lets_a_live_init_finish_with_its_lock`, `a_repository_with_a_head_keeps_its_lock_files`, `an_agora_that_cannot_be_made_says_why` (a real deadline-killed init), `a_git_init_a_kill_left_unfinished_is_reported_and_the_boot_goes_on` |
| **A killed harness loses nothing it wrote.** Open waits out a live git's `index.lock`, then removes a stale one; the reconcile commits what was left, before the boot writes anything | `Committer::open`, `Agora::open` | `a_harness_killed_at_each_point_of_a_commit_loses_nothing`, `a_harness_killed_at_random_during_commits_loses_nothing` (R1.3), `what_a_killed_harness_left_is_reconciled_under_its_own_subject_first` |
| **Retention loses no task.** The whole ledger is validated before anything moves. Done tasks past the threshold are appended to the archive before the trimmed ledger is written, so a write cut short leaves a task in both, never in neither; an archive that cannot be written leaves the ledger as it was; and no kept task loses a dependency | `eph_agora::retention::split`, `Agora::write_tasks` | `a_done_task_a_kept_task_depends_on_stays_however_far_back_the_chain_runs`, `a_ledger_past_its_threshold_archives_the_old_done_first_then_trims`, `a_ledger_write_validates_the_whole_ledger_before_it_archives`, `an_archive_that_cannot_be_written_leaves_the_ledger_as_it_was` |
| **Every degradation is visible.** A failure short of the committer at boot (a stale or stuck `index.lock`, a killed `git init`'s lock files, a repository made over existing files, a damaged roster or ledger, a render, seed, reconcile or commit that fails, a torn, unreadable or unwritable book) is returned as a condition in the degradation grammar, never a silent default (invariant 7), and carried on the degradation channel (R1.4b) | `Agora::open`'s `Booted::conditions`, `eph_agora::degradations::Degradations` | `a_stale_index_lock_is_removed_at_open_and_reported`, `a_repository_made_over_existing_files_says_its_history_is_gone`, `a_damaged_roster_or_ledger_is_reported_and_left_as_evidence`, `a_reconcile_that_cannot_commit_is_reported_and_the_boot_goes_on`, `a_seed_that_cannot_commit_is_reported_and_stays_on_disk`, `a_torn_book_is_reported_at_boot_and_its_line_left_as_evidence` |
| **A line the caller was told about is on disk, and was published after it** (R1.4b). One lock per append; the line is synced, then sent to every subscriber in file order; a dropped receiver unsubscribes itself, and no subscriber can block or fail an append | `eph_agora::book::Book::append` | `appends_are_stamped_in_order_and_read_back_from_a_cursor`, `a_line_is_published_after_it_is_on_disk_to_every_live_subscriber_in_order`, `a_fault_stops_an_append_before_its_write_or_after_it_unpublished` |
| **A kill mid-append loses nothing acknowledged and tears nothing before the last line** (R1's exit; Architect, 2026-10-25). A read skips the live file's unterminated tail without consuming it, counts damage everywhere else, and the next append starts on a fresh line. Proved over every byte prefix an append can leave, and by real kills of a child writer at its fault points and at random (GYM-014) | `Book::since`, `Book::tail`, `eph_fs::append::AppendLog` | `every_prefix_a_killed_append_can_leave_loses_and_tears_nothing_before_it`, `a_writer_killed_at_each_point_of_an_append_loses_nothing_acknowledged`, `a_writer_killed_at_random_loses_nothing_it_acknowledged`, `a_torn_live_tail_is_skipped_unconsumed_and_read_whole_once_finished` |
| **A cursor survives rotation**: a byte offset into the segments and the live file read as one stream; a read lists the segments and opens the live file under the writer's lock, then reads unlocked, so a rotation during a read changes nothing it sees | `Book::since`, `Book::tail` | `a_cursor_taken_before_a_rotation_points_at_the_same_line_after_it` |
| **The degradation channel cannot fail and cannot flood** (2026-09-03, carried): one entry per cause in a ring of 50, a line at the first occurrence, every power of ten and the clear; a line the book cannot take is noted, a skipped rotation reported, neither recursing; a restart replays the book's last 400 lines as carried | `Degradations` | `a_cause_is_one_entry_its_repeats_counted_and_the_ladder_writes_its_lines`, `the_ring_keeps_fifty_causes_and_drops_the_oldest_however_noisy_it_was`, `a_line_the_book_cannot_take_is_noted_without_another_append`, `a_skipped_rotation_is_reported_and_its_own_skip_only_counted`, `a_replay_carries_what_held_drops_what_cleared_and_never_replaces_what_is_live` |
| **Late is not lost** (Architect, 2026-10-25): a commit that fails raises an `agora/commit` cause, and every such cause clears at the next commit that lands, its clear saying how long it held. The committer reports each landing before it answers the batch's tickets | `eph_git::committer::Hooks::on_landing`, `Agora::open`'s hooks | `every_landing_is_reported_before_its_tickets_are_answered`, `every_agora_commit_cause_clears_at_the_next_commit_that_lands` |
| **Work an agent's ignore file hides is named, and attributes change nothing** (Architect, 2026-10-25): what git ignores beyond the harness's two shapes is `agora/hidden-work` until nothing is; the Agora's git reads attributes from the empty tree | `Committer`'s batch status, `judge_ignored`, `eph_git::runner::Git::agora` | `work_an_agents_own_ignore_file_hides_is_named_until_nothing_is`, `a_boot_with_no_landing_still_checks_what_git_ignores`, `a_directory_named_like_a_temporary_file_is_committed_and_the_file_is_not`, `the_agora_reads_no_attributes_from_the_tree` |

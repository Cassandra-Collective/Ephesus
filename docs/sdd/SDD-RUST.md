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

The checklist is the 2026-10-19 inventory of every file the TypeScript build persists:
about 45 files and directories across 13 areas. The 36 `schemaVersion` modules of
`src/shared/` each map to a row of it, or to the three bundled art-pack manifests (R13).

**First principle: two roots, split by trust and by lifecycle.** The **home** is
app-local and owner-only. It holds what the harness alone reads, what an agent must not
see, and what changes faster than a commit. The **Agora** is the git repository with
one committer (ADR-0004, P3). It holds what agents read and write, and the
coordination record whose history is the audit trail. Today the split exists, but the
home's state files sit loose at its root beside the Architect's policy files, so their
writers and lifetimes are mixed. The design groups them by who writes them.

```
<home>/                       ~/.ephesus, or $EPH_HOME (unchanged: the one override)
  policy/                     ARCHITECT-AUTHORED, harness reads, never writes after seeding
    settings.<ext>            what config.json's Architect fields were (§4.1)
    gates.<ext>               the gate policy (was gate-policy.json)
    authority.<ext>           Artemis's delegated authority (was authority.json)
    github-app.<ext>          the company identity (was github-app.json)
  state/                      HARNESS-WRITTEN durable records, all through one store (§4.2)
    company.json              mode, consent and their history pointers (was in config.json)
    activations.json          live mission instances (ADR-0027)
    gates.json                open gates and settled verdicts
    drafts.json               outbound drafts held by their gates (ADR-0030)
    triggers.json             trigger id → last fired
    breaker-stops.json        durable rung-3 stops
    known-targets.json        targets offered for activation (a convenience)
  secrets/                    0700; the broker's store (§4.3; its backend is R6's memo)
  run/                        PRESENT ONLY WHILE A HARNESS RUNS
    control.sock | pipe name  the Architect's front door (ADR-0033)
    events.sock  | pipe name  the agents' endpoint
    endpoint.json             where control.sock is, and which process holds the home
  db/
    ledger.sqlite             the durable cost ledger and app-local tables (§4.10)
  index/                      DERIVED, disposable: rebuilt when damaged (P5, SDD §10)
    fts.sqlite                the keyword rung
    mempalace/                MemPalace's store (external, P1)
  usage/<agentId>.json        engine usage reports, written by eph-usage (§4.4)
  prompts/                    OVERRIDES ONLY: a file here replaces the bundled one (§4.4)
  profiles/<name>/            installed profile bundles (ADR-0012)
  instances/<instance>/       frozen copies of an approved bundle's playbooks (M8b.1)
  tools/<name>/               Architect-owned tool directories granted by name (M8.7b)
  engines/<engine>/<agent>/   each hire's private engine install (ADR-0026; external, P1)
  worktrees/<agentId>/        git worktrees of the agent's target (external, P1)
  scratch/stoa/<src>@<pin>/   read-only study checkouts (R11)
  DIAGNOSIS.md                what works, what does not, and why (M8.13)

<home>/agora/                 the Agora: a git repository, one committer (ADR-0004)
  .gitignore                  written by the harness: excludes mail/*/inflight/ (§4.5)
  PROTOCOL.md                 the agent-facing contract (bundled; see §4.6)
  roster.json                 who is hired (was registry.json)
  tasks.json                  the task ledger
  board.md                    the blackboard, one scribe
  log/
    live.jsonl                the book of record's open segment (was log.jsonl)
    <firstSeq>.jsonl          closed segments (was log-archive/log-<firstSeq>.jsonl)
  mail/<address>/             one mailbox per address: an agent id, or `human` (§4.5)
    new/  inflight/  done/  refused/   delivery states, moved by rename
    outbox/                   an agent's outgoing mail (agents only)
  agents/<agentId>/
    identity.md               rendered at every spawn
    memory.md                 long-term memory (Library layer 1)
    memory-archive/<date>-<n>.md
  knowledge/<name>.md         the shelf (FR-6.4)
  odeon/                      briefs/ decks/ memos/<id>/ minutes/ retros/ (unchanged)
  gymnasium/                  LEDGER.md, proposals/ (shapes fixed by FR-12.6)
  stoa/                       watchlist.json, briefs/ (FR-13.7)
```

**Departures from today's layout, with reasons:**

| Change | Because |
|---|---|
| `policy/` and `state/` split out of the home's root | Inventory P4. `config.json` held Architect-edited fields beside harness-written ones, and `saveConfig` wrote defaults over a damaged file. One writer per file, grouped by writer, makes "who may overwrite this" a property of the directory. |
| `run/` for sockets and the endpoint file | Present only while a harness runs, so `run/` absent or empty means "no harness". Today the home's root mixes transient and durable files. |
| `db/ledger.sqlite` instead of `db.sqlite` | Named for what it holds. Its tables are §4.10. |
| `index/` rebuilt when damaged | SDD §10 promised it and the TypeScript build never did it (inventory P9). Derived state needs no rescue (ADR-0027 §5). |
| `prompts/` holds overrides only | Inventory P6. A prompt copied on first read never receives a shipped improvement. Memory: *seeded only when absent means design the absence.* |
| `agora/log/` with `live.jsonl` and segments beside it | One directory for the book of record. Today the live file and its archive sit in two places. |
| `agora/mail/<address>/` instead of `agents/<id>/inbox|outbox` and `human/inbox` | Mail is one subsystem (Hermes), and the Architect's mailbox was a special case outside SDD §2 (inventory P9). One layout serves every address. |
| `roster.json` instead of `registry.json` | It records who is hired, not a registry of live processes (§4.5). |
| `cursor.json` removed | No consumer: the `done/` and `inflight/` directories are the authority (inventory §11, P7). |
| `.gitignore` excludes `mail/*/inflight/` | A message in flight is a transient state the book of record already logs. Committing every move doubled the commit traffic with no audit value. |

## 4. Persisted formats

### 4.0 Rules every format follows

These turn P4 to P9 into mechanics. A row in §4.1 to §4.11 states only where it departs
from them.

1. **One version field:** `"schemaVersion": <int>` as the first key of every JSON
   document and of every JSONL line, and `PRAGMA user_version` in every SQLite file. A
   file has exactly one; nested versions inside records are removed (inventory §11).
   An unknown version is DAMAGED, and an older one is read only through a named
   migration (P4).
2. **One time format:** RFC 3339 in UTC with milliseconds (`2026-10-19T12:34:56.789Z`)
   for every timestamp in every file. Today some files use ISO strings, some use epoch
   milliseconds, and some filenames use ISO with `:` and `.` replaced. A filename that
   carries a time uses the same instant in compact form (`20261019T123456789Z`).
3. **One id grammar:** `<kind>-<compact UTC time>-<8 lowercase hex>`, where kind is
   `msg`, `task`, `gate`, `memo`, `meet`, `brief`, `draft` or `act` (activation). Ids
   sort by time, are filename-safe, and an agent can mint one for a message without a
   library. Today there are five grammars (inventory §3.7, §5).
4. **One reference grammar:** an artifact reference is a path relative to the Agora's
   root (`odeon/decks/…`, `mail/agent.x/done/…`). A log entry is referred to as
   `log:<seq>`, and a message as `msg:<id>`. Today decks are Agora-relative while
   briefs and minutes are home-relative (inventory P8), and `task.source.log` is
   documented as one thing and written as another (P7).
5. **Two write paths:** `eph_fs::write_atomic` (temp, flush, fsync, rename, with the
   Windows rename retry) for anything another process reads, and
   `eph_fs::AppendLog` for the append-only files (P6).
6. **One store for structured records:** `eph_agora::Store<T>` validates on read and
   **before every write**, distinguishes ABSENT from DAMAGED, and takes a per-record
   **on-damaged policy**:
   - `Refuse`: the record reads as damaged, and writes are refused until the Architect
     repairs it, with the evidence left on disk.
   - `Block`: the same, and the dependent action is also refused (the breaker's case).
   - `Rebuild`: derived state is deleted and rebuilt.

   No record is ever written over a damaged file. Today `registry.json` and
   `tasks.json` are written without validation (inventory P3), and `config.json` and
   `watchlist.json` are overwritten when damaged (P4, P5).
7. **Live process state is not stored** (ADR-0027 §5, P7). A field that mirrors what a
   running process is doing (an agent's status, a hook's fidelity, an engine's
   capacity park) is derived at runtime and shown, never persisted.
8. **Seeded files record their origin.** A file the harness writes once from a shipped
   default carries `"seededFrom": "<shipped version>"`. At boot the harness compares
   it with the shipped version and reports drift (`home/policy-behind:<file>`). That
   way, without overwriting the Architect's edits, a shipped improvement is at least
   disclosed (inventory P6).

### 4.1 The Architect's policy: `policy/`

**The format question, put to the Architect with this design:** these four files are
written by the harness once and edited by hand afterwards. The proposal is **TOML**,
which takes comments, so the shipped gate policy can carry a reason above each rule. It
would need `toml` (approved for `xtask`) approved for runtime use too. The alternative
is JSON, as today, with no comments. The rows below hold for either.

| File | Today | Designed | Departs because | Absent / Damaged |
|---|---|---|---|---|
| `policy/settings` | `config.json`'s Architect fields: `mempalaceCommand`, `wakeWordEnabled`, `pacing{slowAtPercent, holdAtPercent, slowWakeGapMs, wakeCapMs}` | `schemaVersion`, `seededFrom`, `memory.mempalaceCommand`, `voice.wakeWord`, `pacing{slowAtPercent, holdAtPercent, slowWakeGap, wakeCap}`. Durations are strings (`"90s"`, `"6h"`), parsed by `jiff` | Split from the harness-written fields (§4.2), because two writers on one file was P4. Fields are grouped by subsystem. Durations are written as durations, so a person cannot misread a millisecond count | Absent: shipped defaults, reported once. Damaged: `Refuse`. The harness runs on defaults, says so, and never writes it |
| `policy/gates` | `gate-policy.json`: `autonomy`, `maxDailyTokens?`, `rules[{kind, autonomy, maxSpendTokens?, channels?, requireRepeatBack?}]` | Unchanged fields, plus `schemaVersion` and `seededFrom`. Rules are keyed by gate kind (`[rules.destructive]`), not an array | A table keyed by kind makes "at most one rule per kind" structural, where today it is a refine check. The fields themselves are right: they are FR-11.1, FR-1.9 and FR-11.7 | Absent: deny-all, reported (FR-11.1: it can only loosen). Damaged: deny-all, `Refuse`. The FR-11.7 ceiling write patches only `autonomy` and `maxDailyTokens`, and refuses over a damaged file (unchanged) |
| `policy/authority` | `authority.json`: `grants[{class, domains, maxSpendTokens?}]` | Unchanged fields, plus `schemaVersion` and `seededFrom` | The shape is FR-5.5's. Absent is now reported, which matches `gates` (inventory P6: one was silent, the other warned) | Absent: no delegated authority, reported once. Damaged: no authority, `Refuse` |
| `policy/github-app` | `github-app.json`: `appId`, `installationId?`, `slug?` | Unchanged fields, plus `schemaVersion` | The shape is ADR-0022's. The secret key stays in the broker | Absent: no company identity, silently (an unconfigured feature). Damaged: no identity, reported |

### 4.2 Harness state: `state/`, all through the one store (§4.0 rule 6)

| File | Today | Designed | Departs because | On damaged |
|---|---|---|---|---|
| `state/company.json` | `config.json`'s `mode`, `everEnabledImproving` and `consent{grantedAt, terms}` | `schemaVersion`, `mode: directed\|improving`, `everImproving: bool`, `consent{grantedAt, terms}` | Harness-written, so it is split from the Architect's settings (P4). The **current** mode lives here. Its history is the book of record (`gym/mode-changed`), and LEDGER.md's *Mode changes* section is rendered from that. Today mode is stored in three places with no stated authority (inventory §11) | `Refuse`: mode reads `directed` (the safe default) and consent reads "ask", both reported. Neither is written over the damaged file; the Architect's next grant asks him to repair it first |
| `state/activations.json` | `{instances: ProfileInstance[]}`, with `plan` verbatim, `crew`, `armed`, `pendingEvents` | Unchanged shape, with ids in §4.0's grammar and times in its format. `armed` is removed: it is derived from `plan.triggers` while `crew = live` | The plan stays verbatim (ADR-0027: never re-derived). `armed` can be computed from the plan and the crew, and storing it was a second copy of that fact (P7) | `Refuse`: the company comes back un-hired, reported with that consequence (unchanged meaning, ADR-0027) |
| `state/gates.json` | `{open: OpenGate[], settled[{id, verdict}] ≤1000}`, with a nested `schemaVersion` in each gate | `open[{id, kind, agent, because, channel, packaging{what, why, blastRadius, rollback}, task\|null, repeatBack, memoTrigger\|null, openedAt}]`, `settled[{id, verdict, settledAt}] ≤1000` | The nested version is removed (rule 1). `settledAt` lets the trim keep the newest by time instead of by id order | `Refuse`: the approvals queue is empty and every blocked task is reported as orphaned (unchanged) |
| `state/drafts.json` | `{drafts: FiledDraft[] ≤1000}`, with a nested `schemaVersion` in each draft | Unchanged shape without the nested version, keyed by `gate` | Rule 1. The refine "no two drafts on one gate" becomes a map keyed by gate id, so it holds by construction | `Refuse`: a restored gate whose draft did not come back is reported (ADR-0030) |
| `state/triggers.json` | `{lastFired: {id → epoch ms}}` | `{lastFired: {id → time}}` | Rule 2 | `Refuse`: every trigger is due, reported with that consequence (unchanged) |
| `state/breaker-stops.json` | `{stops[{agentId, at, signals, detail}]}`, its own store whose `load` throws | Unchanged shape, keyed by agent, through the one store with `Block` | The store's on-damaged policy carries the safety contract that made it a special case: the breaker blocks every start and respawn, as today | `Block` (unchanged in effect) |
| `state/known-targets.json` | `{targets[{profile, target{kind, id, path}, lastUsedAt}] ≤200}`, trimmed to 50 | Unchanged shape, trimmed to 50 | A convenience: what it holds is right | **The one exception to rule 6, stated:** damaged reads as empty and is reported, and the next successful activation rewrites it. Losing it costs a retyped path and nothing else. Written in the store as `Rebuild` |

### 4.3 Secrets, sockets and the endpoint file

| Artifact | Today | Designed | Departs because |
|---|---|---|---|
| `secrets/` | `secrets.enc`: plaintext JSON wrapping base64 Electron `safeStorage` ciphertext, `{secrets: {NAME: {cipher, lastRotated}}}` | `secrets/index.json` `{schemaVersion, names: {NAME: {lastRotated, backend}}}`, plus the values in a backend R6 chooses (the OS keychain is the candidate; a memo row is owed) | Electron's `safeStorage` does not exist in Rust. The `.enc` name described something the file was not (inventory P12). The index carries names and dates, never a value, so listing and rotation reporting need no backend call (invariant 6) |
| `run/control.sock`, `run/events.sock` (Windows: `\\.\pipe\ephesus-{control,events}-<16 hex of sha256(home)>`) | HTTP over a Unix socket or named pipe; routes `/hook`, `/recall`, `/gh-token`, `/control` | **Newline-framed JSON over the same sockets**: one request line, one response line, each carrying `schemaVersion` and `op`. The pipe naming is unchanged. The ops are specified at R3 (hooks) and R7 (control) | Ephesus now owns both ends (the shims and `ephctl` are Rust binaries in this workspace). HTTP would need a crate outside Set A, and framing a line of JSON needs none. The 0600 socket, the per-spawn token and the probe-before-unlink rule are unchanged |
| `run/endpoint.json` | `control-endpoint.json`: `{schemaVersion, endpoint, path, pid, startedAt}`; `startedAt` and `schemaVersion` have no reader | `{schemaVersion, control, pid}` | P7: fields nobody reads are removed. The version is kept (rule 1). Still a courtesy, not a lock: occupancy is decided by probing the sockets (ADR-0034) |

### 4.4 Other home files

| Artifact | Designed | Departs because | Absent / Damaged |
|---|---|---|---|
| `usage/<agentId>.json` | `{schemaVersion, observedAt, agent\|null, fiveHour\|null, sevenDay\|null, session\|null, sessionCostUsd\|null}`, each window `{usedPercent, resetsAt}`. Written by the Rust `eph-usage` through `write_atomic` | Times per rule 2. The writer gains the Windows rename retry, which today's `.mjs` copy dropped (inventory P10) | Absent: normal. Damaged: reported once per detail; the freshest valid report wins (unchanged) |
| `prompts/` | Overrides only. `eph-core` reads the bundled prompt unless `prompts/<same path>` exists. Nothing is copied | P6. A shipped prompt improvement now reaches every home that has not chosen to override it. The Herald's model and voice ids move out of `prompts/herald/*.md` into `policy/settings` (`voice.modelId`, `voice.voiceId`), because they are configuration, not prose (P12) | An override that cannot be read is reported, and the bundled prompt is used |
| `profiles/<name>/` | Unchanged layout. `triggers/*.json` gain `schemaVersion` | P1: the one bundle file without a version | Unchanged: refused with the reason |
| `instances/`, `tools/`, `scratch/` | Unchanged | They hold frozen copies, Architect-owned directories and read-only checkouts; nothing about them was wrong | — |
| `DIAGNOSIS.md` | Unchanged: rendered markdown, no version, never read back | A human-facing render. A version would describe nothing a reader acts on | — |

### 4.5 The Agora: roster, ledger, board, book of record, mail

| Artifact | Today | Designed | Departs because | Absent / Damaged |
|---|---|---|---|---|
| `roster.json` | `registry.json`: `{orchestratorId, agents: {id → {name, role, engine, capabilities, seat, envGrants, profile, target, isOrchestrator?, status?, hookFidelity?, budget?, hire?, spawnedAt?, lastSeen?}}}` | `{schemaVersion, orchestrator: id\|null, agents: {id → {name, role, engine, capabilities, seat, envGrants, instance\|null, cwd, target: ref\|null, budget\|null, hire{template, version}, hiredAt, retiredAt\|null}}}` | Rule 7: `status`, `hookFidelity` and `lastSeen` describe a running process or an engine, so they are derived and shown, not stored. `retiredAt` is the one lifecycle fact routing needs, because mail to a retired agent bounces. `isOrchestrator` duplicated `orchestrator` and is removed (P7). Today `target` held a 4096-character cwd against a 256-character schema (P3), so `cwd` and `target` are separate fields. `hire` is written now; today it is declared and never written (P7) | Absent: an empty roster. Damaged: `Refuse`, so roster writes stop and the evidence stays (unchanged in effect, now through the store) |
| `tasks.json` | `{tasks: Task[] ≤10000}`, with `source{kind, via, log}` | Unchanged shape, plus ids, times and refs per §4.0 (`source: {kind, ref: "msg:<id>"}`) | Rules 3 and 4. The fields are FR-4.3's. Growth is bounded today by a schema cap with nothing at the cap. Archiving done tasks is owed and named here, to be decided at R1.4 | As today, through the store: `Refuse` |
| `board.md` | Free markdown, one scribe | Unchanged | Agent-facing text (P2), one writer (ADR-0005) | Absent: empty |
| `log/live.jsonl` and `log/<firstSeq>.jsonl` | One JSON object per line, `{ts: epoch ms, seq, kind, …open fields}`, no version. Rotated at 4 MiB into `log-archive/` | Every line is `{schemaVersion, seq, at, kind, event, source?, refs?, data}`. `kind` and `event` come from a closed vocabulary per kind, so the writer cannot emit an unknown pair. `data` is the event's typed payload. The reader keeps a line it does not know as an opaque record and counts it, rather than dropping it. Rotation at 4 MiB is unchanged, and closed segments are never rewritten | The book of record is read by the standup, the proof gate and the org metrics. Open fields (the 27 kinds whose events were free strings) meant no reader could know what a line could say (inventory §10, P1). A typed writer and a tolerant reader let the format grow without one release breaking an older log. `seq` stays the identity, and the duplicate-`seq` case cannot arise with one harness per home (ADR-0034) and one appender | Absent: a new log. A torn last line is ignored and left on disk (unchanged, P6). A segment that cannot be opened is reported, and the log continues from the next |
| `mail/<address>/…` | `agents/<id>/inbox/{,.inflight,.done,.rejected}`, `outbox/{,.rejected}`, `human/inbox/{,.done}`, `cursor.json` | `mail/<address>/new/`, `inflight/`, `done/`, `refused/`, and `outbox/` for agents. States move by rename. Each message is one JSON file named `<id>.json` | **Unchanged mechanism:** a rename between state directories is atomic, and a state field inside the file would need a rewrite per transition. **Changed:** one layout for every address, the Architect's included (P9). `rejected` is renamed `refused`, because the log calls the act a refusal and the vocabulary should match. `cursor.json` is removed (P7) | An unparseable incoming message is moved to `refused/` with a teaching reply to its author (unchanged) |
| The message (each mail file) | `{id, conversation, in_reply_to, from, to, act, subject, body, hops, requires_reply, needs_human, created_at}`, with no version, although `MESSAGE_SCHEMA_VERSION` exists | `{schemaVersion, id, conversation, replyTo\|null, from, to, act, subject, body, hops, needsHuman, at}` | Rule 1: agents write messages, so the protocol tells them to write the version, and the validator refuses one without it. `requires_reply` is removed, because it was corrected on read to be a function of `act` (request, query, propose; FR-3.3), which made it two fields that cannot disagree (P7). Field names follow the one casing (camelCase) every other format uses | — |

### 4.6 Per-agent files and the agent-facing contract

| Artifact | Designed | Departs because |
|---|---|---|
| `PROTOCOL.md` | Bundled with the harness and **rendered into the Agora at every boot** | Today it is seeded once and never refreshed, so a protocol change never reached a running company (P6). The Agora's history then shows every protocol change an agent could have read |
| `agents/<id>/identity.md` | Unchanged: rendered at every spawn from the bundled template and the hire's brief | Right as it is |
| `agents/<id>/memory.md` | Unchanged format: markdown sections headed `## <date> — <author>` (ADR-0006). **Changed writing rule:** the harness appends with an append-only open, never a read-modify-write, and condenses it only while the agent is held at a Stop boundary or is down | Today the harness's read-modify-write raced the agent's own appends (inventory P10). Two appenders through append-only opens do not lose each other's lines. Condensation, the one rewrite, waits for the agent to be still |
| `agents/<id>/memory-archive/<date>-<n>.md` | Unchanged: write-once | Right as it is |

### 4.7 The Odeon, the Gymnasium and the Stoa

| Artifact | Designed | Departs because |
|---|---|---|
| `odeon/briefs/`, `decks/`, `memos/<id>/memo.md`, `minutes/`, `retros/` | Unchanged formats (markdown and HTML, write-once). Ids and filenames per §4.0. Refs are Agora-relative | Rules 3 and 4. Human-facing archives whose shape FR-7 fixes |
| `odeon/memos/<id>/verdict.json` | Unchanged fields, plus `schemaVersion` (already present) and times per rule 2 | Right as it is; the FR-5.5 refine stays |
| `gymnasium/LEDGER.md`, `proposals/GYM-*.md` | **Unchanged shapes:** FR-12.6 requires the running system to keep the build phase's artifact shapes. The reader joins each row with its proposal file for the fields the table does not carry | Today the parse is lossy: `class` always reads `craft`, and `proposedBy` reads empty (inventory §6.1). The proposal file already holds them, so the fix is in the reader, not the shape |
| `stoa/watchlist.json`, `briefs/RB-*.md` | Unchanged shapes. The watchlist goes through the store with `Refuse` | Today `register` and `retire` wrote over a damaged watchlist (inventory P5). The Stoa seed decisions of 2026-10-11 (DECISIONS-LOG, carried from PR #76) apply at R11 |

### 4.8 Message bodies an endpoint receives

The ledger's `propose` ops, the Odeon's deck, memo, verdict and brief filings, the
Library's condensation, the Harbor's triage and root-cause reports, the Gymnasium's
proposal and the Stoa's research brief each carry their own `schemaVersion` in the
message body (inventory §3.8). Each is specified by the phase that builds its endpoint
(R5 to R11), under §4.0's rules. R1.0 fixes only that a body payload names its own
version and is refused, with a teaching reply, when it does not validate.

### 4.9 Not in R1.0's scope, and where it is decided

| Format | Phase |
|---|---|
| The hook envelope, recall, and the gh-token request and response on `events.sock` | R3 |
| The control socket's ops | R7 |
| The secret backend | R6, with its crate memo |
| Window geometry | R4 (egui's own persistence, or a row in `db/ledger.sqlite`) |
| The floor's art-pack manifests (`*.tileset.json`, `*.characters.json`, `*.emotes.json`: bundled assets, not home files) | R13 |

### 4.10 SQLite

| Database | Designed | Departs because |
|---|---|---|
| `db/ledger.sqlite` | `PRAGMA user_version = 1`, WAL. Tables: `cost_ledger(id, agent, session, model, day, inTokens, outTokens, costUsd NULL, source)` append-only, as FR-11.2 requires; `cost_fold_cursor(agent, source, folded)`; `installed_settings(agent, path, backup NULL, createdDir)` | Rule 1: today neither database has a version (P1). The tables are the right ones. SDD §4.6's `command_history` and `metrics_rollup` never existed, and are not designed in until a phase needs them (P9). A row that fails validation is reported and skipped (unchanged) |
| `index/fts.sqlite` | `PRAGMA user_version`; `doc_state`, `passages` (FTS5) unchanged | A damaged or older-version index is deleted and rebuilt (`Rebuild`), as SDD §10 always said (P9) |

### 4.11 External formats, read as they are (P1)

| Artifact | Owner | Read for | Ground truth |
|---|---|---|---|
| `engines/claude/<agent>/eph-settings.json` | Claude Code's settings schema. Ephesus writes it, in the engine's format | Hooks, statusLine, permissions, attribution keys | `test/fixtures/engine-output/`, memory *Claude Code engine ground truth* |
| `engines/claude/<agent>/.claude.json` | The engine, which rewrites it wholesale | Onboarding and workspace trust (ADR-0021, ADR-0025), merged in | As above |
| `engines/claude/<agent>/projects/<slug>/<session>.jsonl` | The engine's transcript | Token and cost facts for the ledger (FR-11.2), the capacity tail | `test/fixtures/claude-limit/` |
| `worktrees/`, the Agora's `.git/` | git | Worktree state, commits | git's documented behaviour (RUST-PLAN §2) |
| `index/mempalace/` | MemPalace | Recall (ADR-0016) | Its CLI, version-probed |

### 4.12 Today's format problems, each closed or carried

| Inventory problem | Closed by |
|---|---|
| P1 missing versions (messages, log, triggers, markdown, SQLite) | Rule 1. Markdown human artifacts are the stated exception: they carry no version because no program parses them back, except LEDGER.md, whose shape FR-12.6 fixes |
| P2 loose or shared version constants | One named constant per format, in `eph-schema` |
| P3 roster and tasks written without validation | Rule 6 |
| P4 config overwritten when damaged | `policy/` and `state/` split, rule 6 |
| P5 watchlist overwritten when damaged | Rule 6 |
| P6 seeded files never refresh | Prompts become overrides, PROTOCOL.md is rendered at boot, and seeded policy records its origin (rule 8) |
| P7 fields nobody reads, values nobody writes | `cursor.json`, `startedAt`, `requires_reply`, `isOrchestrator`, `status`, `hookFidelity` and `armed` removed; `hire` written |
| P8 inconsistent refs | Rule 4 |
| P9 doc drift | This section is the layout. SDD.md stays the frozen tree's |
| P10 non-atomic writers | One write path (rule 5). Agent appends to memory are append-only and condensation waits |
| P11 lossy derived cursors | Out of R1.0's scope. Each is decided where its subsystem is built (the standup's `lastSeq` at R9, MemPalace's gate at R8) |
| P12 misleading formats | `secrets/` renamed and redesigned; the Herald's ids moved to settings |
| P13 unbounded growth | Named, not designed: `mail/*/done/`, `refused/`, `memory-archive/`, `odeon/**` and `cost_ledger` grow without bound today. A retention rule for each is owed at its phase, with the done-task archive at R1.4 |

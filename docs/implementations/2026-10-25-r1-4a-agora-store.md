# R1.4a — `eph-agora`, the stores: the validated store, the roster, the ledger and its retention, the board, and the boot

**Dates:** 2026-10-20 (built and reviewed) and 2026-10-25 (closed) · **Branch:**
`feature/r1-4a-agora-store`, cut from R1.3's tip `8c57d28` · **Package:** R1.4a
(`docs/RUST-PLAN.md` R1; split from R1.4 by the Architect, 2026-10-20) · **Design:**
ADR-0004 (the single committer), FR-4.1, FR-4.2, SDD-RUST §2, §4.0 rule 6, §4.5, §4.6 and
§10 (written and approved this package) · **Decisions:** DECISIONS-LOG:
- 2026-10-20, *R1.4 — Architect decisions*: the split, done-task retention, severity
  deferred to R4;
- 2026-10-25, *R1.4a — Architect decisions*: §10 approved, the four departures from the
  committed SDD text ratified, the archive's writer with no `seq`;
- 2026-10-25, *R1.4a — Architect decisions, second round*: the store re-reads, the
  ledger's operations land with their callers, the committer detects nested ignores;
- 2026-10-25, *R1.4a — implementation*.

**Gymnasium:** GYM-013 (the rewrite)

## 1. Problem / motivation

R1.1 typed the roster and the ledger, R1.2 gave the home one write path, and R1.3 gave the
Agora one committer. Nothing yet read a record back or decided what a damaged one means,
and nothing opened the Agora the way a booting harness must. The TypeScript build had
three defects of exactly that shape, recorded in SDD-RUST's inventory:

- **P3:** `registry.json` and `tasks.json` were written without validation.
- **P4 and P5:** `config.json` and the Stoa watchlist were written over when damaged, so
  the evidence of the damage was destroyed by the next write.
- **P6:** `PROTOCOL.md` was seeded once and never refreshed, so a protocol improvement
  never reached a company that had booted before.

SDD-RUST §4.0 rule 6 answers all three with one store that tells *absent* from *damaged*,
validates before every write, and never writes over damage. R1.4a builds it, the two
Agora records that go through it, the board, the task ledger's retention (SDD-RUST §4.5
left it owed; the Architect decided it on 2026-10-20), and the boot order that seeds and
reconciles the Agora.

The same package had to keep a promise R1.1 made: `tasks.json` is validated as a whole
ledger, so the retention that trims it must never leave a kept task depending on one it
moved out.

## 2. What changed

| File | Change |
|---|---|
| `crates/eph-agora/` | New crate, depending on `eph-schema`, `eph-fs` and `eph-git` only, with `serde` for tests. 3 modules, 40 tests. |
| `…/src/store.rs` | `Store<T>`: `load` answers `Absent`, `Present` or `Damaged`; `save` validates, then refuses over damage; the Refuse, Block and Rebuild policies; `Damage` with its cause and consequence. |
| `…/src/retention.rs` | `split`: the pure retention rule, with its two numbers as parameters for small tests. |
| `…/src/agora.rs` | `Agora::open`, the boot order; the roster, ledger and board accessors; `write_tasks` with the retention and the archive; the owned-file render and the seeds. |
| `crates/eph-schema/src/tasks.rs` | `ArchivedTasks`, the archive's line, sharing the ledger's per-task rules (`Task::check`) and its version. |
| `crates/eph-schema/src/document.rs` | A byte-order mark is refused, with the fix in the message. |
| `crates/eph-fs/src/home.rs` | `home::prompt`: the home's override of a bundled prompt (SDD-RUST §4.4), with an unreadable one reported. |
| `crates/eph-fs/src/append.rs` | `append_line`, an append-only line with no `seq` and no rotation; `AppendLog::append` shares its `write_line`. |
| `crates/eph-git/src/committer.rs` | Open finishes a `git init` a kill left unfinished: with no `HEAD`, it waits out and then removes the lock files a killed init left directly in `.git`, and names them in `Opened::init_locks`. Two R1.3 tests no longer race on Linux. 3 new tests. |
| `Cargo.lock` | The new member. No new external crate. |
| `docs/sdd/SDD-RUST.md` | §2's `.gitignore` wording; §4.5's rows for the archive and the `.gitignore`, and the board's override; §4.6's `PROTOCOL.md` row; §4.12's P13; §10, the recovery invariants, approved. |
| `docs/RUST-PLAN.md`, `docs/PROGRESS.md` | R1.4 split into R1.4a and R1.4b; `DIAGNOSIS.md` moved to R7. |
| `docs/DECISIONS-LOG.md` | The R1.4 entry of 2026-10-20 and the three R1.4a entries of 2026-10-25. |

## 3. Implementation approach

### 3.1 The store

`Store<T>` is a path, a policy and a phantom type. It is generic over `eph_schema`'s
`Versioned`, which supplies the file name, the version and the validator, so one store
serves every structured record a later phase adds.

- **`load`** first asks for the path's own metadata (`symlink_metadata`), not its
  target's. Not found is the only answer that reads as absent. A link to nothing exists,
  so it is read, fails, and is damaged. A read that fails is damaged too. Text is parsed by
  `eph_schema::document::parse`, the strict reader R1.1 built: a key written twice, an
  unknown field, a byte-order mark, a version this build does not read, or a failed
  validation is damaged, with the reader's own words as the reason.
- **`save`** renders, and so validates, first. A value that fails its own validation is
  never written. It then calls `writable`, which loads again and refuses while the file is
  damaged under Refuse or Block, then writes through `write_atomic`.
- **Rebuild** is for derived records. A damaged file whose text was read is deleted when
  it is read, because what is lost is known to be invalid. A file that could not be read
  at all is left in place, because what it holds is unknown, and the rebuild writes over
  it. `Damage::deleted` says which.
- **Block** behaves as Refuse inside the store. The policy travels with the `Damage`, and
  refusing the action that depends on the record is the caller's job (R6, for breaker
  stops).
- Nothing is cached (Architect, 2026-10-25), so a repair by hand is seen at the next read.

### 3.2 The boot

`Agora::open(root, prompts, hooks)` runs six steps. Only the first can fail the boot.

1. **The committer** (`eph-git`): its OS lock, `git init` when there is no repository,
   and a stale `index.lock`. A repository made over a directory that already held files is
   reported as `agora/history-lost`, because whatever history it had is gone.
2. **`.gitignore`, rendered**, before anything is committed. It keeps
   `agents/*/inbox/.inflight/` and `write_atomic`'s temporary name out of history.
3. **The reconcile**: whatever a killed harness left is committed under the committer's
   own reconcile subject. It is skipped only for a repository this boot made over an empty
   directory, which has nothing to reconcile.
4. **`PROTOCOL.md`, rendered** from the home's override at `prompts/agora/PROTOCOL.md`, or
   else the bundled copy (`include_str!` from the repository's `prompts/`, invariant 8).
5. **The seeds, only when absent**: an empty roster, an empty ledger, and the board (the
   home's override, or the bundled text). A damaged roster or ledger is reported and left
   as evidence.
6. **One commit, when anything was written**, under `seed the Agora`. When the reconcile
   failed, its leftovers are in this commit too, and the subject names them.

A file the harness owns is rewritten only when it is not a regular file holding exactly
the right bytes. A link is replaced, because git does not follow a `.gitignore` that is a
link. Every failure after the committer becomes a `Condition` in the degradation grammar
(`agora/<slug>[:<subject>]`). `Booted::conditions` carries them until R1.4b's degradation
channel does.

### 3.3 The ledger and its retention

`write_tasks` runs in this order:

1. It validates the whole ledger.
2. It checks the store is writable.
3. It splits the ledger under the retention rule.
4. It validates the kept half.
5. It appends the archived half as one line.
6. It writes the kept ledger.

The fourth step holds by construction, but checking it before the first write means a
fault in the split refuses the write. Otherwise it would archive tasks the ledger then
keeps too. A write cut short between steps 5 and 6 leaves the moved tasks in both files,
never in neither, and an archive that cannot be written leaves the ledger as it was.

The archive is written by `eph_fs::append::append_line`, which reads only the file's last
byte to see whether a killed write left a torn line. The first build opened `AppendLog`,
which recovers a `seq` by reading the whole file, for a file that has no `seq`. The
mutation round showed it (§6), and the Architect chose the narrower writer.

The board is free text with one scribe (FR-4.2). It reads as empty when absent, which is
not a degraded state, and is written whole, ending with a newline.

## 4. Mathematical / statistical details

**The retention rule.** Let $L$ be the ledger, $|L| \le 10{,}000$ (R1.1's cap). Let $D$
be its done tasks and $O = L \setminus D$ the rest. The thresholds are
$A = 9{,}000$ (`ARCHIVE_PAST`) and $K = 1{,}000$ (`DONE_KEPT`).

- If $|L| \le A$, nothing moves.
- Otherwise, order $D$ newest first by $(\text{updatedAt}, \text{id})$, both descending,
  and let $D_K$ be its first $K$. The seed set is $S = O \cup D_K$. The kept set is the
  closure of $S$ under "depends on":
  $$\text{Kept} = \{\, t \in L : t \in S \ \lor\ \exists s \in S,\ s \to^{+} t \,\}$$
  where $s \to^{+} t$ means a chain of `deps` from $s$ reaches $t$. The archived set is
  $L \setminus \text{Kept}$, which holds only done tasks, since $O \subseteq S$.
- Both halves keep the ledger's order. The closure is a graph walk with a visited set, so
  it costs $O(|L| + \sum_t |\text{deps}(t)|)$, and a cycle cannot loop it, though the
  ledger's validation already refuses a task that depends on itself.

So after a trim $|\text{Kept}| \ge |O| + \min(K, |D|)$, and no kept task can name a
dependency outside the ledger, which is exactly what R1.1's whole-ledger validation
demands.

**The archive's cost.** One `AppendLog::append` re-opens the file for the rotation check
and the torn tail, then appends and syncs. At one line per task, the 2026-10-20 session
measured 8,001 appends at 62 s, about $62{,}000 / 8{,}001 \approx 7.75$ ms a line. That
was the debug test profile, on win32, under `%TEMP%`. Defender's real-time protection was
on when checked on 2026-10-25; its state at the measurement was not recorded. R1.2's
single-open loop costs 1.02 ms a line. At one line per run, the cost is one synced append
however many tasks move. With `append_line`, it also no longer grows with the archive's
size.

## 5. Design decisions

| Decision | Chosen | Rejected, and why |
|---|---|---|
| What fails a boot | Only the committer's refusal (Architect, §10) | Failing on a render or a seed would stop a company over a file it can do without, and invariant 7 asks for a report, not a stop |
| Boot order | `.gitignore` before the reconcile, the reconcile before anything the boot writes | The first build reconciled first: a killed write's temporary file reached history. Seeding first would call crash leftovers a seed |
| `.gitignore` | Owned, rendered at every boot, the exact temporary name (Architect, 2026-10-25) | Seeded once: an edit or a link could widen or empty it. `.*.tmp`: an agent's own `.notes.tmp` would never be committed |
| The archive's line | One line per retention run (Architect, 2026-10-25) | One line per task: 62 s for 8,001 tasks |
| The archive's writer | `append_line`, no `seq` (Architect, 2026-10-25) | `AppendLog` with a `seq` reader nothing uses: two equivalent mutants and a whole-file read on every write. An exclusion would have kept both |
| Rebuild's deletion | Only text known to be invalid | Deleting a file that could not be read destroys what nobody has seen |
| Caching | None (Architect, 2026-10-25) | A cache hides a repair by hand until it is invalidated |
| `PROTOCOL.md`'s text | Today's, until R5 (Architect, 2026-10-25) | Correcting the message fields now would change `prompts/agora/PROTOCOL.md`, which the frozen TypeScript build also ships (`src/main/agents.ts:1024`), and teach its agents a message its validator refuses; and no Rust agent writes a message before R5 |
| The ledger's operations | With their callers, R2.3 to R9 (Architect, 2026-10-25) | Porting all ten now, tested on fixtures only |
| An agent's own ignore file | The committer detects it, in R1.4b (Architect, 2026-10-25) | A grant forbidding the names stops only the file tool |
| Exemptions | None: `eph-agora` writes through `eph-fs` and runs git through `eph-git`'s committer | Its own `fs-write:` claim would widen what `invariants` lets through |
| A `git init` a kill left holding `config.lock` (found by R1.4a's Linux run) | Open finishes it: with no `HEAD`, it waits over the retry schedule, removes every lock file directly in `.git` except its own and `index.lock`, runs `git init`, and the boot reports `agora/init-lock` (Architect, 2026-10-25) | Refusing with the fix in the message, which keeps the boot fatal until a person acts; a residual for R7 |
| R1.3's two Linux-racing tests | Fixed on this branch, production unchanged for the races (Architect, 2026-10-25) | A separate `fix/` branch; recorded flakes with reruns |

**What the reviews changed** (2026-10-20, a design-fidelity review and an adversarial
refutation pass, both read-only, before any mutation round; fixed in `40c1fe6`):

- The ledger is validated whole before the split. An archive line holds each task to the
  ledger's own per-task rules, and its version is the ledger's.
- A file beginning with a byte-order mark is refused with the fix. It had read as a shape
  error with no word about why.
- The home's prompt overrides are honoured (`eph_fs::home::prompt`). The first build read
  only the bundled text, against SDD-RUST §4.4.
- `.gitignore` is rendered before the reconcile and narrowed.
- A failed render or seed is a condition, not a failed boot. The boot commits when it
  wrote something, and names the reconcile when that failed.
- The store never deletes a file it could not read, reads a link to nothing as damaged,
  and says whether a Rebuild deleted.

**Recorded residuals:**

- Ledger writes are not serialised. Two callers writing at once each validate and write
  whole, so the file is never torn, but one write wins. The callers arrive with R5 and R7.
- Records have no size cap beyond R1.1's 10,000 tasks.
- A hand edit of `PROTOCOL.md` is reverted at the next boot without a report.
- `Store` accepts any `Versioned` type, a JSONL line type included, which it would read as
  one document.
- `save` does not create its directory. The Agora's root always exists by the time a
  record is written.
- No phase reads `tasks-archive.jsonl` yet. Its reader must dedupe by id and let the
  ledger win (SDD-RUST §4.5).
- The `cfg(unix)` tests (a link to nothing, a delete refused by directory permissions, an
  owned file that is a link) run on POSIX only (§6).

**Corrected record.** R1.3's committed implementation entry (DECISIONS-LOG 2026-10-20,
item 15) says R1.4 *seeds* the Agora's `.gitignore` with `.*.tmp`. It is rendered at every
boot and narrowed. The correction is in R1.4a's implementation entry, and R1.3's entry is
not edited.

## 6. Verification

Two machines' worth of evidence, each with its condition. **Windows:** Windows 11 Home
10.0.26300, toolchain 1.96.0-x86_64-pc-windows-msvc, git 2.53.0.windows.2, Defender's
real-time protection on. **Linux:** WSL2 on the same machine, kernel 6.6.114.1, 20 CPUs,
uid 1000, the same toolchain, git 2.53.0, the tree copied from `git ls-files -co
--exclude-standard` into the WSL home, with its target directory there too. Tier A unless
marked.

**The gate.** On the final tree, fmt, clippy, test, deny, invariants and deps are green
on both machines. Coverage fails only for the new crate's floor, as designed (R1.1):
`eph-agora` measures 96.64% on Windows and 95.97% on Linux. Every other crate is at or
above its floor on both: `eph-fs` 97.48 and 97.98 against 97.24, `eph-git` 99.09 and 98.96
against 98.92, `eph-schema` 98.92 against 98.87, and `xtask` 98.26 against 98.26. The
frozen tree's checks (`prettier --check .`, `eslint .`, `check-invariants.cjs`,
`check-readme-current.cjs`, `check-attribution.cjs`) pass on Windows.

**The tests that carry the claims** are the ones SDD-RUST §10 cites, each named with the
invariant it proves. The four that run on POSIX only passed on Linux:
`an_owned_file_that_is_a_link_is_replaced_by_a_regular_one`,
`a_link_to_nothing_is_damaged_not_absent_and_is_never_deleted`,
`a_derived_record_that_cannot_be_deleted_says_so` (a delete refused by directory
permissions, as a non-root user, so the probe ran), and
`a_prompt_override_that_links_to_nothing_is_reported`.

**What the Linux run found.** R1.3's four Linux CI runs had passed. Here:

| Condition | Before the fixes | After `bb4baf5` | After `4fb469d` |
|---|---|---|---|
| `cargo test -p eph-git --lib`, repeated | 5 of 18 runs failed | 0 of 18 | — |
| `cargo llvm-cov --workspace`, repeated | — | 2 of 22 failed | 0 of 12 |
| `cargo test --workspace`, repeated | — | — | 0 of 12 |

Of the two failures under instrumentation, one was the test's: the deadline fired after
`git init` had written a whole repository. A probe killed `git init` 360 times at 0 to
3 ms. It left nothing 40 times, only `config` once, and all of `HEAD`, `config`,
`objects/` and `refs/`, a valid repository, 319 times. The other was the defect §5
records, a killed `git init`'s `config.lock`, with git's own words:
`could not lock config file …/.git/config: File exists`.

**Mutation rounds** (`cargo xtask mutants <crate>`, Windows, each with its control: one
shard with no test run, which must miss every viable mutant). No round had a TIMEOUT, and
each reported its sources unchanged.

| Crate, at | Mutants | Caught | Missed | Unviable | Control | Verdict |
|---|---|---|---|---|---|---|
| `eph-agora`, `e4f85d4` | 90 | 48 | 2 | 40 | 15 of 15 missed | survivors (below) |
| `eph-agora`, `c3debff` | 88 | 48 | 0 | 40 | 14 of 14 | ROUND OK |
| `eph-fs`, `c3debff` | 101 | 89 | 0 | 12 | 26 of 26 | ROUND OK |
| `eph-schema`, `c3debff` | 319 | 276 | 0 | 43 | 72 of 72 | ROUND OK |
| `eph-git`, `bb4baf5` | 155 | 103 | 0 | 52 | 32 of 32 | ROUND OK |
| `eph-git`, `4fb469d` (the final tree) | 164 | 112 | 0 | 52 | 29 of 29 | ROUND OK |
| `eph-agora`, `4fb469d` (the final tree) | 89 | 49 | 0 | 40 | 15 of 15 | ROUND OK |

`eph-fs` and `eph-schema` did not change after `c3debff`, so their rounds stand for the
final tree.

The first round's two survivors were `no_seq -> Some(0)` and `Some(1)`: equivalent,
because nothing read the archive's `seq`. The Architect chose `append_line` over an
exclusion, and the next round killed or removed every mutant.

**A slip in the method.** The first rounds' `cargo xtask` build ran without
`CARGO_TARGET_DIR`, so 258 MB of build output landed in a git-ignored `target/` inside
OneDrive, in two worktrees. Nothing tracked changed. This worktree's was removed with
`cargo clean`, and every later build exported the variable. The other worktree's is left
for the Architect.

**CI** (tier B). Run 40165646997 on `b2318cb`, three attempts:
- **The steps.** On each attempt, every TypeScript job passed. The Rust gate on Linux,
  macOS and Windows passed fmt, clippy, test, deny, invariants and deps, and failed
  coverage only for `eph-agora`'s missing floor, by design.
- **The figures.** `eph-agora` measured 95.97% on Linux and macOS and 96.64% on Windows
  in every attempt, the same figures as the local runs on both machines.
- **The floor.** `cargo xtask coverage ratchet` over the nine artifacts seeded
  `eph-agora` at 95.97%. It raised `eph-fs` to 97.48%, `eph-git` to 98.96% and
  `eph-schema` to 98.92%, and lowered nothing, on tree `fnv1a64:81712cd8a87bd1c7`. With
  it, `cargo xtask gate` exits 0 on Windows, `coverage: ok (5 crate(s), 38 production
  file(s), windows)`.

## 7. Related docs

- ADR-0004 (the single committer); ADR-0034 (one harness per home, R7).
- `docs/RUST-PLAN.md` R1; `docs/sdd/SDD-RUST.md` §2, §4.0, §4.5, §4.6, §10.
- `docs/implementations/2026-10-20-r1-1-eph-schema.md`: the strict reader and the
  whole-ledger validation this package's retention keeps.
- `docs/implementations/2026-10-20-r1-2-eph-fs.md`: `write_atomic` and `AppendLog`.
- `docs/implementations/2026-10-20-r1-3-eph-git.md`: the committer and its reconcile.
- `src/main/agora.ts`, `src/shared/ledger.ts`: the TypeScript behaviour carried, and
  where it was changed.

# R1.4b — `eph-agora`, the book of record: the log, its readers, publish and subscribe, the degradation channel, and a kill mid-append

**Date:** 2026-10-25 · **Branch:** `feature/r1-4b-book-of-record`, cut from R1.4a's tip
`3fe7490` · **Package:** R1.4b (`docs/RUST-PLAN.md` R1; split from R1.4 by the Architect,
2026-10-20) · **Design:** ADR-0004 (the single committer), FR-4.1, FR-4.2, FR-4.4, NFR-7,
NFR-13, invariants 5 and 7, SDD-RUST §4.5 and §10 (edited and approved this package) ·
**Decisions:** DECISIONS-LOG, 2026-10-25:
- *R1.4b — Architect decisions*: a channel per subscriber, a byte-offset cursor, one lock
  for the writer, `agora/commit` cleared at the next landing;
- *second round*: the ignore check folded into the batch's status, no attributes from the
  tree, the archive reader built now, every `agora/commit` cause cleared at a landing,
  `agora/log-torn`, the archive reader's shape, a kill proved by plant and by real kill;
- *third round*: a directory named like a temporary file re-included, the harness's own
  two shapes matched instead of asking git which file ignored a path;
- *fourth round, after review*: a nested repository kept out and named, `.git/` outside
  an agent's grant, a boot clears the boot causes it no longer finds, `agora/log-size`,
  the channel as the one writer of degradation lines, links recorded not named, the SDD
  edits approved;
- *R1.4b — implementation* (the minor decisions, and the outcomes below).

**Gymnasium:** GYM-013 (the rewrite); GYM-014 (`eph-agora` may start its own test binary),
filed before the code.

## 1. Problem / motivation

R1.4a gave the Agora its records and its boot, and returned every boot failure short of
the committer as a `Condition` that nothing yet carried. The book of record itself, the
append-only `log.jsonl` that FR-4.4 makes the one source of truth for the activity view
and the briefings, had a writer in `eph-fs` (R1.2) and no reader, no subscriber and no
degradation channel. R1's exit owed four things this package had to supply:

- FR-4.4's book, written by one writer and read back across rotation;
- invariant 7's channel, so a give-up becomes a visible state and survives a restart;
- "killed mid-append, nothing lost or torn on reopen", the reader's half of R1.2's writer;
- the committer's detection of work an agent's own ignore file hides (the Architect's
  answer of 2026-10-25 at R1.4a).

The TypeScript build is the behavioural reference (`src/main/eventlog.ts`,
`src/main/degradations.ts`, `src/main/agora.ts`), with two of its shapes deliberately not
carried: its reader paged by `seq`, which a second harness on one home repeats, and it
published inside the appending thread, where a slow listener slowed every append.

The package's reviews then found three defects that predate it and were fixed in it, each
on the Architect's answer: one agent's `git init` in its own directory stopped every Agora
commit, or hid its work as a gitlink (R1.3's committer); the rendered `.gitignore` hid any
directory named like a temporary file, and everything in it (R1.4a); and in-tree
attributes could fail every `add -A` or rewrite line endings in history.

## 2. What changed

| File | Change |
|---|---|
| `crates/eph-agora/src/book.rs` | New. `Book`: one writer behind one lock, opened late if it cannot open now; `append` stamps, syncs, publishes; `subscribe`; `since(cursor)` and `tail(n)` with their counts; `ends_mid_line`, `size_bytes`; the fault seam; the planted-prefix and real-kill tests. |
| `…/src/degradations.rs` | New. `Degradations`: the channel carried from the TypeScript build: a ring of 50 causes, the ladder, report, clear, `clear_where`, has, list, replay; a line the book cannot take is noted, a skipped rotation reported, neither recursing. |
| `…/src/archive.rs` | New. The reader of `tasks-archive.jsonl`: every archived task the ledger does not hold, the newest line winning, with the book's line counts. |
| `…/src/agora.rs` | The boot order gains the book's replay, `agora/log-torn`, the channel's reports after the reconcile, the clear of boot causes not found again, the record-only commit and the boot's own ignore check; the committer's hooks gain the Agora's reporter and landing report; `harness_ignores`; `judge_ignored`; `append`, `subscribe`, `since` (`agora/log-size`), `tail`, `archived`. The `.gitignore` re-includes a directory named like a temporary file. |
| `…/src/lib.rs`, `…/Cargo.toml` | The new modules; `serde_json` from crate Set A. |
| `crates/eph-git/src/committer.rs` | `Hooks::on_landing`, reporters that add up; the batch's status in porcelain v2, listing what git ignores and flagging gitlinks; `Ignored`; `Committer::ignored`; nested repositories kept out through the committer's block of `.git/info/exclude`, a gitlink unstaged, the batch retried. |
| `crates/eph-git/src/runner.rs` | The Agora's profile passes `--attr-source` at the empty tree; an old git's refusal names the fix; `.git/` stated as the line. |
| `xtask/src/invariants.rs` | `eph-agora` may claim `process:` (GYM-014), with a test that its neighbours still may not. |
| `Cargo.lock` | `eph-agora` depends on `serde_json`. No new crate. |
| `docs/sdd/SDD-RUST.md` | §4.5's rows for the book, the archive, the `.gitignore`, `.git/info/exclude` and attributes; §10's new invariants. Approved by the Architect, 2026-10-25. |
| `docs/RUST-PLAN.md` | §3.2: `eph-git` and `eph-agora` claim `process:` in their tests. |
| `docs/gymnasium/` | GYM-014 and its ledger row. |
| `docs/DECISIONS-LOG.md` | Four Architect entries and the implementation entry. |

## 3. Implementation approach

### 3.1 The book

One `Book` per Agora holds one `Mutex` over the `AppendLog` writer and the subscribers'
senders. An append takes the lock, stamps the next `seq` and the time while `AppendLog`
renders the line, lets `AppendLog` sync it, and only then sends it to each subscriber's
own `mpsc` channel, dropping a sender whose receiver is gone. The lock is held across
the send, so receivers see the file's order. A send on an unbounded channel never blocks,
so no subscriber can slow or fail an append; a receiver nobody reads grows its queue, the
recorded residual the Architect accepted.

A reader takes the same lock only to list the sealed segments with their lengths and to
open the live file, then reads unlocked. A rename never moves an open file, and a sealed
segment is never renamed again, so a rotation during a read changes nothing the read sees.
A cursor is one byte offset into the segments, oldest first, then the live file. Each
segment is read to its listed end and consumed whole, its unterminated last line counted
as damage; the live file's unterminated last line is skipped and not consumed, so the
next read sees it whole. `tail(n)` walks the sources backwards and stops as soon as it
holds `n` records, and returns the cursor `since` would end at.

### 3.2 The channel

`Degradations` carries the TypeScript build's shape, cited rather than re-decided (the
Architect, 2026-09-03; DECISIONS-LOG 2026-10-25): one entry per cause, refreshed in place,
in a ring of 50 that drops the oldest; a line at the first occurrence, every power of ten
and the clear; a replay of the book's last 400 lines as carried, a cleared row removing its
cause and a live report replacing a carried one. Its lock order is the ring, then the
book's writer, and nothing holds either while it waits on a commit, so the committer's
thread may report into it. A line the book refuses is noted in the ring as
`agora/log-unwritable` without another append, and a skipped rotation is raised as
`agora/log-rotation`, whose own skipped rotation is only counted. The channel is the one
writer of `degradation` lines: `Agora::append` refuses one.

### 3.3 The boot

The order (SDD-RUST §10) is: the book's replay and its own conditions (a torn, unreadable
or unwritable book), all reads; the committer, with the caller's hooks plus the Agora's;
the `.gitignore` render; the reconcile; the conditions found so far onto the channel, so
no line of this boot enters the last run's reconcile; the `PROTOCOL.md` render; the
seeds; the clear of every boot-checked cause not found again; one commit when anything was
written, the book included; and the ignore check once more when no batch landed.

### 3.4 What the committer learns at each landing

The batch already runs `git status` after `git add -A`. It now runs it as porcelain v2
with `--ignored=matching`, in the same walk: an entry other than `!` decides whether
anything is staged, a `!` entry is ignored, and a changed entry whose `sub` field begins
with `S` is a gitlink. The landing report carries the ignored entries and the nested
repositories kept out, before any ticket of the batch is answered. The Agora clears every
`agora/commit` cause on it and names in `agora/hidden-work` every ignored entry that is
not one of the harness's own two shapes, and every nested repository, as a repository.

A nested repository is handled on the failure or gitlink path only. When `add -A` fails,
`git status --porcelain=v2 --untracked-files=all` lists each untracked directory git
would not descend into, which is a nested repository; when the status lists a gitlink,
that path is one too. Either is written into the committer's block of
`.git/info/exclude`, git's wildcards escaped, any other line of the file kept; a gitlink
is unstaged with `rm --cached -f`; the batch retries. A listed path whose `.git` is gone
leaves the list before the next batch, and the list in memory changes only once the file
holds it.

### 3.5 The archive reader

`Agora::archived` reads `tasks-archive.jsonl` against the ledger: a damaged ledger refuses
the read, since which tasks it holds is unknown. Each whole line that parses as an archive
line contributes its tasks; a repeated id keeps its first place and its newest content,
and a task the ledger holds is left out.

## 4. Mathematical / statistical details

**The cursor.** With sealed segments $S_1, \dots, S_k$ of lengths $\ell_1, \dots, \ell_k$
(sorted by the first `seq` each holds) and the live file $L$, the book is the stream
$S_1 \Vert \dots \Vert S_k \Vert L$. A cursor $c$ addresses byte $c$ of it: segment $j$
covers $[b_j, b_j + \ell_j)$ with $b_j = \sum_{i<j} \ell_i$, and the live file starts at
$b_{k+1}$. A rotation renames $L$ to $S_{k+1}$ with $\ell_{k+1} = |L|$, so every offset
below $b_{k+1} + |L|$ addresses the same byte before and after, and $c$ is stable.

**The ladder.** A cause's $n$th occurrence writes a line iff $n \in \{10^0, 10^1, \dots\}$,
so $N$ occurrences write $\lfloor \log_{10} N \rfloor + 1$ raise lines, and one clear.

**The prefixes a kill can leave.** An append writes $t = [\texttt{\textbackslash n}]\,
\textit{line}\,\texttt{\textbackslash n}$ in one `write_all`, so after a kill the file is
$p \Vert t[..k]$ for some $0 \le k \le |t|$ ($p$ the earlier bytes). The plant test runs
every $k$, twice: after whole lines, and after a torn line (when $t$ begins with
`\n`). With $j = k - [\text{torn before}]$ bytes of the new line written, the record
survives iff $j \ge |\textit{line}|$ (its object is whole, newline or not), and the damage
counted after the next append is $[\text{torn before}] + [0 < j < |\textit{line}|]$.

**The cost of the ignore check.** Measured before the build (DECISIONS-LOG 2026-10-25,
second round, item 1): 3,003 agent files, win32, median of 21 runs. A batch 285 ms; a
separate `ls-files --others --ignored --exclude-standard` 71 ms (about 28% of a batch);
the batch's own status with `--ignored=matching` 101 ms against 103 ms without it.

## 5. Design decisions

The Architect's answers are in the DECISIONS-LOG entries above. Choices made inside them,
approved with the SDD edits (fourth round, item 7), and the residuals the package leaves:

- **A boot that wrote only degradation lines commits them** under `RECORD_SUBJECT`
  (`RECONCILE_AND_RECORD_SUBJECT` when the reconcile failed). A failure of that commit is
  still `agora/commit:seed`: the cause names the boot's own commit, whatever it holds,
  since renaming a cause already in a book is a migration.
- **The boot's conditions reach the channel after the reconcile**, not as each is found,
  so no line of this boot enters the last run's reconcile. The second round's item 4
  wrote "as they are found"; the intent, that a later landing in the same boot clears
  them, holds.
- **The causes** `agora/log-unreadable`, `log-unwritable`, `log-torn`, `log-rotation`,
  `log-size`, `hidden-work` and `ignore-check`. `agora/log-unwritable` is both the boot's
  condition and the channel's note of a line it could not write; a note never replaces
  an entry's wording, so the boot's fuller account stays.
- **A detail is cut at 2,000 characters**, saying so, and an empty one replaced, so every
  row validates. `MAX_DETAIL` repeats `eph-schema`'s rule, and a test holds the two to
  one number.
- **`Hooks` reporters add up** instead of replacing, a change to R1.3's contract, so the
  Agora's reporter never silences a caller's.
- **The book's replay runs before the committer opens**: it only reads, and a second
  harness is still refused by the committer's lock before anything is written.
- **The fault seam** (`AppendPoint::BeforeWrite`, `AfterWrite`) sits in the production
  path, as the committer's does, so the real-kill test can stop a child at each point.
- **Recorded residuals:** a receiver nobody reads grows its queue (until R4); a line a
  landing's report writes reaches history at the next commit; a hand-shortened live file
  leaves a cursor past its end, and the next whole read after it regrows lands mid-line;
  two segment names that parse to one `first_seq` sort by `read_dir` order (hand-planted
  only); a nested repository whose path holds a line break cannot be listed and still
  fails `add -A`; a clean gitlink already in history is taken out only at its first
  change; a directory whose `.git` is re-created after its files were tracked is untested;
  a symbolic link is committed as the link, its target outside the Agora (R2.3); `.git/`
  is outside an agent's grant (R2.3); `tasks-archive.jsonl` grows without bound, by the
  Architect's retention design.

## 6. Verification

Two machines' worth of evidence, each with its condition. **Windows:** Windows 11 Home
10.0.26300, toolchain 1.96.0-x86_64-pc-windows-msvc, git 2.53.0.windows.2, Defender's
real-time protection on (checked 2026-10-25), the target directory under
`%LOCALAPPDATA%`. **Linux:** WSL2 on the same machine, kernel 6.6.114.1, 20 CPUs, uid
1000, cargo 1.96.0, git 2.53.0, cargo-llvm-cov 0.9.1, the tree copied with `git archive`
of the commit into the WSL home, its target directory there too, and checked file for
file against `git ls-tree` (1081 of 1081 identical). Tier A unless marked.

**The gate** at `5bab4ec`, the code-final commit (later commits change only records):
all seven steps green on both machines.

| Crate | Floor | win32 | Linux (WSL2) |
|---|---|---|---|
| `eph-agora` | 95.97% | 99.21% (1013 of 1021) | 98.82% (1009 of 1021) |
| `eph-fs` | 97.48% | 97.48% | 97.98% |
| `eph-git` | 98.96% | 99.30% (1003 of 1010) | 99.20% (1001 of 1009) |
| `eph-schema` | 98.92% | 98.92% | 98.92% |
| `xtask` | 98.26% | 98.26% | 98.26% |

Two earlier gate runs were red at coverage alone (`eph-agora` 95.62%, then `eph-git`
98.61%): the new code's failure arms were unrun. Tests reached them, two unreachable
branches were removed, and no floor was lowered or raised. The frozen tree's checks
(`prettier --check .`, `eslint .`, `check-invariants.cjs`, `check-readme-current.cjs`,
`check-attribution.cjs`) pass on Windows, and nothing under `src/`, `test/`, `shims/` or
`scripts/` changed.

**The repeated runs** (GYM-014 metric 2; RUST-GOAL step 4), at `5bab4ec`, the whole
workspace each time, a run with no test result counted as a failure:

| Condition | win32 | Linux (WSL2) |
|---|---|---|
| `cargo test --workspace --locked` | 0 of 10 failed | 0 of 10 failed |
| `cargo llvm-cov --workspace --locked --no-report` | 0 of 10 failed | 0 of 10 failed |

The first win32 plain series failed 10 of 10, every failure naming a deleted copy's
workspace root: a GYM-014 refutation had built `xtask` from a scratch copy into the
shared target directory, and cargo left that copy's `debug/xtask.exe` in place. After
`cargo clean -p xtask`, the series was run again in full. An early series at `1664131`,
before the review fixes, was also 0 of 10 and 0 of 10 on Linux.

**The real kills.** `a_writer_killed_at_each_point_of_an_append_loses_nothing_acknowledged`
stops a child at `BeforeWrite` (nothing on disk) and `AfterWrite` (the line on disk,
never acknowledged); `a_writer_killed_at_random_loses_nothing_it_acknowledged` kills a
child 0, 40, 90, 160 and 250 ms after its first acknowledged line. After every kill,
every acknowledged line reads back, nothing before the last line is damaged, the reopen
raises `agora/log-torn` exactly when the live file ends mid-line, and the next append is
whole. On Windows a kill rarely lands inside one small write, so there the planted
prefixes carry the torn cases: `every_prefix_a_killed_append_can_leave_loses_and_tears_nothing_before_it`
plants every byte prefix of one append, after whole lines and after a torn line.

**The reviews**, both read-only, before any round:
- *Design fidelity:* no code defect; nine gaps and eight nits, all fixed or recorded in §5
  (a stale module contract, GYM-014's `agora/log-torn` after real kills, the landing
  report's order proved only by a race, a rotation during reads untested, concurrent
  appenders untested, §10's citations, the slow read's name, `Agora::append` bypassing the
  channel); four questions, put to the Architect (fourth round).
- *Refutation:* two defects reachable from an agent's own directory, both in R1.3's
  committer (a nested repository's wedge and its gitlink), fixed on the Architect's
  answer; `.git/info/attributes` and `.git/config` recorded as outside an agent's grant;
  lock order, the boot's ordering and the clears held; two hand-tamper gaps recorded.

**Mutation rounds** (`cargo xtask mutants <crate>`, win32, each with its control: one
shard with no test run, which must miss every viable mutant). Every reported round had
its sources unchanged.

| Crate, at | Mutants | Caught | Missed | Timeout | Unviable | Control | Verdict |
|---|---|---|---|---|---|---|---|
| `eph-agora`, `186fb7f` | 305 | 219 | 9 | 4 | 73 | 67 of 67 missed | INVALID |
| `eph-agora`, `3eb16eb` | 292 | 217 | 0 | 1 | 74 | 63 of 63 | INVALID |
| `eph-agora`, `6c168b9` | 292 | 219 | 0 | 0 | 73 | 63 of 63 | ROUND OK |
| `eph-git`, `6c168b9` | 218 | 161 | 1 | 0 | 56 | 50 of 50 | survivor |
| `eph-git`, `5bab4ec` | 218 | 162 | 0 | 0 | 56 | 50 of 50 | ROUND OK |
| `xtask`, `5bab4ec` | 517 | 484 | 0 | 0 | 33 | 120 of 120 | ROUND OK |

Round 1's four timeouts were real hangs: `archive::read` and `book::consume` advanced by
index arithmetic, which a mutant turned into an endless loop; both now walk
`split_inclusive`. Its nine survivors: eight killed by tests or removed with the code
they tested, and one, `replace > with >= in Book::since`, excluded in
`.cargo/mutants.toml` on the Architect's answer (it opens a segment the cursor has passed
and reads nothing from it). Round 2's one timeout was a test, not a hang: under the
mutant that stopped the count, the ring test's 3,000 repeats each wrote a synced line, so
it repeats 200 times. `eph-git`'s survivor, a deleted `NESTED_END` arm, showed no test put
a line after the committer's block. `eph-agora`'s sources did not change after
`6c168b9`. `eph-fs` and `eph-schema` did not change in R1.4b, so R1.4a's rounds stand.

**GYM-014 metric 1**, refuted on scratch copies of `5bab4ec`, each with its own target
directory: a `process:` exemption planted in `eph-agora` passes `cargo xtask invariants`;
the same plant fails it in `eph-schema` (`src/lib.rs:43`, "may not claim `process:`", and
`no-io` at 47 and 48) and in `eph-fs` (`src/lib.rs:22`).

**CI** (tier B). Run 40206703962 on `237aa31` is green on every job: the Rust gate on
`ubuntu-latest`, `macos-latest` and `windows-latest`, all seven steps; the TypeScript
`Typecheck · lint · test`; `Commit attribution`; `Docs integrity`. Its coverage figures
are the local ones exactly: Linux and macOS as WSL2, Windows as win32. The runners' git:
2.55.0 (ubuntu), 2.56.0 (macOS), 2.56.0.windows.2 (Windows), each at least the 2.40
`--attr-source` needs.

**Measured to decide, before the build** (DECISIONS-LOG second and fourth rounds): the
ignore check's cost (§4), the attributes probe, and the nested-repository probes, each on
git 2.53.0.windows.2, the attributes and links also on WSL's git 2.53.0.

**Not proven:**

| Claim | Where | Evidence | What remains |
|---|---|---|---|
| A kill mid-append tears nothing before the last line | win32, Linux | A: every planted prefix; real kills at fault points and at random | A kill that lands inside one `write_all` is rare on Windows, so the torn states there rest on the plant; macOS has CI's single pass only (B) |
| The kill tests hold on loaded machines | win32, Linux | A: 10 plain and 10 instrumented runs each | Any other machine's timing; CI shows one pass per platform (B) |
| `--attr-source` works on every git a user runs | git 2.53 local, 2.55 and 2.56 in CI | A, B | A git older than 2.40 was not at hand; its refusal text is git's wording for an unknown option, assumed |
| Nested repositories are kept out | win32 git 2.53 | A, and CI's suites (B) | A path holding a line break; a `.git` re-created after its directory's files were tracked |
| `.git/` is outside an agent's reach | — | D: the Architect's boundary | Enforced by R2.3's grants, not yet built |
| GYM-014 metric 3 (the exemption stays in tests) | — | Not yet measured | Due at R2's close |

## 7. Related docs

- ADR-0004 (the single committer); ADR-0034 (one harness per home, R7).
- `docs/RUST-PLAN.md` R1; `docs/sdd/SDD-RUST.md` §4.5, §10.
- `docs/implementations/2026-10-20-r1-2-eph-fs.md`: `AppendLog`, the writer this reads.
- `docs/implementations/2026-10-20-r1-3-eph-git.md`: the committer this extends.
- `docs/implementations/2026-10-25-r1-4a-agora-store.md`: the boot this extends.
- `docs/gymnasium/proposals/GYM-014-eph-agora-may-start-its-own-test-binary.md`.
- `src/main/eventlog.ts`, `src/main/degradations.ts`, `src/main/agora.ts`: the behaviour
  carried, and where it was changed.

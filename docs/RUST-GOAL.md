# The Rust rewrite goal prompt — a handover

Paste the block under [The prompt](#the-prompt), whole, as one `/goal` command, in a
session started in the worktree that has `feature/r1-4a-agora-store` checked out. That
working tree holds an uncommitted draft the first step finishes
([R1.4a, step 1](#r14a-the-owed-steps-in-order)).

This is a **handover**, like [`M9-GOAL.md`](./M9-GOAL.md): it says where the rewrite
stands, what is already decided, and what comes next. The block is the **condition** the
goal evaluator holds the session to. It carries the Architect's direction of 2026-10-19
(ADR-0041): rebuild every part of the system in Rust, verify that it works, integrate and
test it, then move to the next part, and when everything built so far runs in Rust,
continue with the plans that follow. It replaces nothing. The standing prompt for any
session, `docs/GOAL.md`, is on the pushed and unmerged branch `docs/standing-goal-prompt`.

*Written 2026-10-25, when the Architect paused the session after R1.4a's review fixes.*

---

## Where the rewrite stands

| Package | State | Where |
|---|---|---|
| R0.1 workspace skeleton | Merged | #102 |
| R0.2 the gate chain | PR open, mergeable, waiting for review | #104, `feature/r0-2-gate-chain` |
| R0.3, R1.0, R1.1, R1.2, R1.3 | Done, pushed, stacked on R0.2; each gets its PR as the one below it merges | `feature/r0-3-standards-learn-rust` … `feature/r1-3-eph-git` @ `8c57d28` |
| **R1.4a** `eph-agora`, the stores | **Code and both reviews' fixes committed; local only, not pushed** | `feature/r1-4a-agora-store` @ `40c1fe6` |
| R1.4b `eph-agora`, the book of record | Not started | — |

R1.4a's commits, on top of R1.3's tip `8c57d28`:

- `4d19f54` the Architect's R1.4 decisions;
- `a6176a5` `ArchivedTasks`, the archive's line;
- `81adc75` `eph-agora`: `Store<T>`, the registry, the ledger with its retention, the board, and the boot;
- `40c1fe6` the fixes from both reviews.

What `40c1fe6` changed:

- The ledger is validated whole before the split. The ledger and each archive line share one per-task rule set, and `ArchivedTasks::VERSION` is `Tasks::VERSION`.
- A file that begins with a byte-order mark is refused with the fix.
- `eph_fs::home::prompt` applies the home's prompt overrides (SDD-RUST §4.4).
- The boot order is: committer, `.gitignore` render, reconcile (skipped for a repository made over an empty directory), `PROTOCOL.md` render, seeds, then one commit when anything was written. That commit is `RECONCILE_AND_SEED_SUBJECT` when the reconcile failed.
- Only the committer's refusal stops a boot. Every later failure is a condition. A repository made over existing files reports `agora/history-lost`.
- `.gitignore` is narrowed to `write_atomic`'s `.<name>.<8 hex>.tmp`.
- The store:
  - never deletes a file it could not read;
  - reads a link to nothing as damaged;
  - says whether a Rebuild deleted.

**Verified on this machine (tier A, Windows 11, 2026-10-20):** `cargo fmt --check`,
clippy with `-D warnings`, `cargo xtask invariants`, and the tests of `eph-schema` (87),
`eph-fs` (34) and `eph-agora` (40).

**Not run anywhere yet:**

- the full `cargo xtask gate` (stopped at the pause);
- the mutation rounds;
- the `cfg(unix)` tests:
  - a prompt override or a record that is a link to nothing;
  - a delete refused by directory permissions, with its privileged-user probe;
  - an owned file that is a link.

## R1.4a: the owed steps, in order

1. **Finish `docs/sdd/SDD-RUST.md`** (uncommitted in this worktree). The draft already
   has the archive's one-line-per-run wording, the P13 row and a new §10 table.
   - **In §10:**
     - It cites `a_reconcile_or_seed_that_cannot_commit_is_reported_…`. That test is now two: `a_reconcile_that_cannot_commit_is_reported_and_the_boot_goes_on` and `a_seed_that_cannot_commit_is_reported_and_stays_on_disk`.
     - Add that only the committer's refusal stops a boot.
     - Add that Block is enforced by the caller.
     - Add the Rebuild rule: invalid text is deleted when read; a file that cannot be read is left and written over; the damage says which.
     - The boot-order row puts `.gitignore` before the reconcile.
     - The retention row also cites `a_ledger_write_validates_the_whole_ledger_before_it_archives` and `an_archive_that_cannot_be_written_leaves_the_ledger_as_it_was`.
   - **New §4.5 rows:**
     - `.gitignore`: owned, rendered at every boot before the reconcile, the narrow pattern, no override, and replaced unless it is a regular file with exactly the right bytes, because git does not follow a linked `.gitignore`.
     - `tasks-archive.jsonl`: Absent, Damaged, Writer and Readers. An archive that cannot be written refuses every ledger write that would archive. A reader dedupes by id, and the ledger wins.
   - **§4.6:** `PROTOCOL.md`'s row and the board row name the override.
   - **The `.gitignore` wording:** §2 (lines 139 and 160) and PROGRESS's Resume-here block still say `.*.tmp` and "R1.4 seeds the file". Make them say it is rendered at every boot and narrowed. PROGRESS's "Four constraints carry forward" lists three.
2. **Append R1.4a's implementation entry to `docs/DECISIONS-LOG.md`.**
   - **The minor decisions:**
     - the store caches nothing;
     - Block behaves as Refuse inside the store;
     - the boot order and its commit rule;
     - owned files versus seeds;
     - `PROTOCOL.md` is today's text, and R5 corrects it;
     - the archive gets one line per run;
     - the ledger is validated before archiving;
     - the ledger's pure operations stay with their callers;
     - `eph-agora` claims no exemption;
     - §10 is begun.
   - **The review outcomes:** what `40c1fe6` fixed.
   - **The 62 s figure, with its condition.** 8,001 one-line appends took 62 s, about 7.8 ms per line. That was the debug test profile, on win32, under `%TEMP%`. Each `AppendLog::append` re-opens for the rotation check and the torn tail, then appends and syncs. Compare it with `eph-fs`'s 1.02 ms from a single-open loop. Confirm the antivirus state before naming it.
   - **The residuals:**
     - `AppendLog::open` reads the whole archive on every run.
     - Ledger writes are not serialized.
     - Records have no size cap.
     - A hand edit of `PROTOCOL.md` is reverted without a report.
     - `Store` accepts a JSONL type.
     - `save` does not create its directory.
     - The agent-nested ignore question below.
   - R1.3's committed entry said "seeds" for `.gitignore`. Correct it in this entry; never edit that one.
3. **Write `docs/implementations/<date>-r1-4a-agora-store.md`**, with the global standard's
   seven sections.
4. **Run the mutation rounds** over `eph-agora`, `eph-schema` and `eph-fs` (`home::prompt` is
   new there). Each one runs `cargo xtask mutants <crate>`.
   - Check free RAM first. An `eph-git` round was invalid at 366 MB free.
   - The control must miss every viable mutant, and any TIMEOUT makes the round invalid.
   - Change no source during a round.
   - Each exclusion goes in `.cargo/mutants.toml` with its reason.
5. **Run the `cfg(unix)` tests before pushing** if WSL is at hand. Build the copy from
   `git ls-files -co --exclude-standard`. Otherwise CI runs them first, and the report
   says so.
6. **Push and seed `eph-agora`'s coverage floor.** The first push is red at the coverage
   step by design.
   1. Run `gh run rerun <run> --failed` twice.
   2. After each attempt, run `gh run download <run> -p 'rust-coverage-*' -D <dir>/attempt-N`.
   3. Run `cargo xtask coverage ratchet <dir> > floors.new`, then move `floors.new` over `xtask/coverage-floors.json`.
   4. Commit, push, and cite the green run.
7. **Close R1.4a.**
   - Tick it in PROGRESS with the run ID.
   - Point the Resume-here block at R1.4b.
   - Add R1.4a to README's `<!-- landed: … -->` marker and its paragraph.
   - Update the session's memory.

## Then

- **R1.4b**, on its own branch from R1.4a's tip. RUST-PLAN gives its scope:
  - the log writer;
  - its readers across rotation;
  - publish and subscribe;
  - the degradation channel, which then carries `Booted::conditions` and the committer's `Hooks::reporting_to` failures as `agora/commit` rows;
  - the reader half of killed mid-append.

  Ask its open choices first.
- **R1's exit** (RUST-PLAN):
  - FR-4.1, FR-4.2, FR-4.4 and NFR-7 by test;
  - nothing lost or torn on reopen after a kill mid-append or mid-commit;
  - a mutation round with a control;
  - every test that runs git does so hermetically.

  Close it with `/milestone-review`.
- **R2 and every later phase** start only after the Architect approves that phase's package
  list. At parity (R14) the TypeScript tree is deleted, and whether M9 may start is asked.

## Settled: cite these, do not re-ask

- **2026-10-19**
  - ADR-0041: the rewrite, window included.
  - The Architect approved RUST-PLAN, crate Set A (RUST-CRATES), GYM-013's thresholds and the SRS wording.
  - Each package gets one branch and one PR, stacked.
- **2026-10-20, R1.0**
  - SDD-RUST §2 and §4 are approved.
  - A Rust home carries `home.json`.
  - Changed formats are `schemaVersion` 2.
- **2026-10-20, R1.3**
  - The committer holds its own OS lock.
  - The Agora's git is hermetic.
  - The git-kill defect is recorded now and fixed in R2.
  - A detached worktree whose commit nothing holds is refused, naming the sha.
  - The TypeScript data loss is patched on `fix/worktree-untracked-files`.
- **2026-10-20, R1.4**
  - R1.4 is split into R1.4a and R1.4b, and `DIAGNOSIS.md` moves to R7.
  - Retention: past 9,000 tasks, the done tasks beyond the newest 1,000 done move to the archive, except every dependency of a kept task.
  - Severity is deferred to R4.

## Waiting on the Architect

- **Open the TypeScript fix PR.** Its body was drafted outside the tree, in a temporary
  file named `pr-body-worktree-untracked.md`. If that file is gone, redraft it from
  `docs/implementations/2026-10-20-worktree-untracked-files.md`, which is on that branch
  only.

  ```bash
  gh pr create --base main --head fix/worktree-untracked-files --title "fix(worktrees): never remove untracked work a repo's config hides" --body-file <the body file>
  ```
- **Review #104.** Each stacked branch then gets its PR in turn. Each one hits the
  DECISIONS-LOG append-against-append conflict; resolve it main-first.
- **Ask in the next question batch:** an agent's own `.gitignore` or `.gitattributes` under
  `agents/<id>/` can hide its work from the history. Should the committer detect it and
  raise a condition, or should the agent's grant forbid those files (R2.3)?

## The prompt

```text
/goal Work as the Ephesus senior software engineer on the Rust rewrite (ADR-0041). First read .claude/skills/ephesus-engineer/SKILL.md, docs/RUST-GOAL.md, docs/RUST-PLAN.md, docs/PROGRESS.md's Resume here block, the DECISIONS-LOG entries from 2026-10-19 on, docs/sdd/SDD-RUST.md and the open PRs. Then resume at RUST-GOAL.md's first owed R1.4a step, finish R1.4a, and carry on through RUST-PLAN's packages in order: rebuild each part in Rust, verify it, integrate it, test it, then move to the next. Before a package's code, ask the Architect through AskUserQuestion every choice its documents leave open; a choice already in docs/adr/ or DECISIONS-LOG is cited, not re-asked. Consult the advisor before each package's approach and before calling it done. A package closes with: its owed tests; an adversarial review whose findings are fixed or recorded; a mutation round with no TIMEOUT whose control missed every viable mutant; its branch pushed; CI green on all three OSes, cited by run ID; a new crate's coverage floor seeded from three CI attempts; its PROGRESS tick, DECISIONS-LOG entry and docs/implementations record. Before a package of a phase whose package list the Architect has not approved, ask for that approval. At parity (R14), ask whether M9 may start.
The goal is met when ONE of these is visible in the transcript:
(A) DONE. The Architect ended the session, in an AskUserQuestion answer or a message, and the engineer then printed a GOAL REPORT marked DONE in the skill's format, covering every package touched, where: every owed test and acceptance criterion is PASS with the exact command and an excerpt of its output from this session; `cargo xtask gate` and `node scripts/check-attribution.cjs` exited 0 in the final turn, output shown, except that a crate whose floor is not yet seeded may fail the coverage step alone, named; each branch's `git status -sb` is shown in sync with its remote; and a "Not proven" section gives, per claim, the platform, toolchain, evidence tier (A-D) and what remains unverified.
(B) BLOCKED. The engineer's latest message starts with "EPHESUS-BLOCKED:" and names an Architect decision asked through AskUserQuestion that came back unanswered twice (empty, blank or timed out).
(C) BOUND. 200 goal turns were evaluated and the engineer printed a GOAL REPORT marked PARTIAL. Every turn starts with "Goal turn N of 200".
These must hold throughout, or the goal is not met:
* Every decision (scope, a design choice, a deviation from SDD-RUST, an invariant or an accepted ADR, a crate outside RUST-CRATES Set A, a toolchain change, a merge, any outward-facing or irreversible action) is the Architect's AskUserQuestion answer, never a default or silence; each answer is appended to DECISIONS-LOG before the work that depends on it.
* The TypeScript tree is frozen: nothing under src/, test/, shims/ or scripts/ changes without an Architect answer.
* Accepted ADRs are superseded, never edited; SRS text changes only on an Architect answer; append-only files are only appended to.
* xtask/coverage-floors.json changes only through `cargo xtask coverage ratchet` over CI artifacts, and no floor is lowered.
* One package per branch, feature/r<n>-<slug>, cut from the previous package's tip and pushed to origin; opening a PR is the Architect's step, so the engineer hands over the gh command.
* Every commit's author and committer is MERT EFE ŞENSOY <sensoymertefe@gmail.com>; no agent, session or model name appears in any file, commit message or PR body, and there is no attribution trailer.
* No result is claimed for a platform or tree not run in this session; a CI result is cited with its run ID and commit, as tier B.
* Builds use a CARGO_TARGET_DIR outside OneDrive, and no source changes during a mutation round.
```

## Its length

The CLI refuses a `/goal` condition longer than **4,000 characters**. It measures the
JavaScript `.length`, newlines included. The condition above, everything after `/goal `,
is **3,764**. Re-measure after any edit, and cut words rather than rules:

```bash
node -e "const t=require('fs').readFileSync('docs/RUST-GOAL.md','utf8');const b=t.split('\`\`\`text\n')[1].split('\n\`\`\`')[0];console.log(b.replace(/^\/goal /,'').trim().length)"
```

## On this machine

- Set `CARGO_TARGET_DIR="$LOCALAPPDATA/ephesus-cargo-target"` in every shell before
  building. `cargo xtask mutants` unsets it itself.
- The name `goal` is shared: the repository has a project skill called `goal` (the
  milestone runner), and the CLI has its own `/goal`. The first time you paste the
  prompt, confirm the CLI answers that a goal is set.

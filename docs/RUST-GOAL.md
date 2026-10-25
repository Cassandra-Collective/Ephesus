# The Rust rewrite goal prompt — a handover

Paste the block under [The prompt](#the-prompt), whole, as one `/goal` command, in a
session whose working tree has `feature/r1-4b-book-of-record` checked out. On
2026-10-25 that is the worktree `amazing-tharp-37716d`. The app's guard refuses edits
in any worktree other than the session's own. So a session that starts somewhere else
must first ask the Architect how to move the branch, as the 2026-10-25 session did:
detach the old worktree, then check the branch out in the new one (DECISIONS-LOG
2026-10-25, *R1.4a — Architect decisions*, item 4).

This is a **handover**, like [`M9-GOAL.md`](./M9-GOAL.md): it says where the rewrite
stands, what is already decided, and what comes next. The block is the **condition** the
goal evaluator holds the session to. It carries the Architect's direction of 2026-10-19
(ADR-0041): rebuild every part of the system in Rust, verify that it works, integrate and
test it, then move to the next part, and when everything built so far runs in Rust,
continue with the plans that follow. It replaces nothing. The standing prompt for any
session, `docs/GOAL.md`, is on the pushed and unmerged branch `docs/standing-goal-prompt`.

*Written 2026-10-20 at R1.4a's pause; rewritten 2026-10-25, when R1.4a closed and R1.4b
began.*

---

## Where the rewrite stands

| Package | State | Where |
|---|---|---|
| R0.1 workspace skeleton | Merged | #102 |
| R0.2 the gate chain | Merged | #104 (`9aa30bd`, 2026-10-25) |
| R0.3, R1.0, R1.1, R1.2, R1.3 | Done, pushed, stacked on R0.2; each gets its PR as the one below it merges | `feature/r0-3-standards-learn-rust` … `feature/r1-3-eph-git` @ `8c57d28` |
| R1.4a `eph-agora`, the stores | **Done and ticked**, pushed. CI run 40167524909 on `ef8bbe1` (the floor) is green on every job; `3fe7490` cites it | `feature/r1-4a-agora-store` @ `3fe7490` |
| **R1.4b** `eph-agora`, the book of record | **Done and ticked**, pushed. CI run 40206703962 on `237aa31` is green on every job; the close commit cites it | `feature/r1-4b-book-of-record`, cut from `3fe7490` |

R1.4a's close is recorded in PROGRESS (its tick and evidence), in
[`2026-10-25-r1-4a-agora-store.md`](./implementations/2026-10-25-r1-4a-agora-store.md),
and in five DECISIONS-LOG entries of 2026-10-25. Two of its findings changed earlier
packages, each on the Architect's answer:
- **A real R1.3 defect.** A `git init` killed mid-write left `.git/config.lock`, and
  every later open failed. Open now finishes the init, and the boot reports
  `agora/init-lock`.
- **A seq-less writer in `eph-fs`.** `eph-fs` gained `append::append_line`, for the task
  archive.

## R1.4b: the owed steps, in order

1. **Ask what is still open, through AskUserQuestion, before any code.** Already
   answered (DECISIONS-LOG 2026-10-25, *R1.4b — Architect decisions*): a channel per
   subscriber; a byte-offset cursor into the archive and live file read as one stream,
   with `since(cursor)` and `tail(n)`; one lock for the writer; `agora/commit` clears at
   the next commit that lands. Still to ask:
   - **The nested-ignore check's cadence.** The Architect decided the committer detects
     work an agent's own `.gitignore` or `.gitattributes` hides, at boot and at each
     commit. First measure what `git ls-files --others --ignored --exclude-standard`
     costs on an Agora with a few thousand agent files. If each commit is too costly, put
     the cadence back to him with the figure and its condition.
   - **Which phase reads `tasks-archive.jsonl`.** SDD-RUST §4.5 says "whichever phase
     builds it". Ask whether it is deferred to the first phase that needs an archived
     task, or built now.
   - Anything else the design raises that no document fixes.
2. **Consult the advisor on the approach**, then build. These are cited, not re-asked:
   - **2026-09-03:** a degradation is its own log kind, deduped by cause, and a restart
     replays what was true when the harness stopped, marked carried.
   - **The TypeScript reference:** publish after the line is on disk, with one
     subscriber's failure isolated (`src/main/agora.ts` `onAppend`). A ring of 50 causes,
     the ladder, report, clear, has, list and replay (`src/main/degradations.ts`). The
     boot replays the last 400 lines (`src/main/index.ts:343`), and a slow full read
     raises a degradation (`readLogSince`).
   - **Already typed:** `eph-schema` types the line (`log.rs`, a typed writer, a tolerant
     reader) and the degradation row with its ladder (`degradation.rs`). Only
     `degradation` lines can be written until each kind's phase defines its events.
3. **What R1.4b builds:**
   - the log writer;
   - its readers across rotation;
   - publish and subscribe;
   - the degradation channel, which takes `Booted::conditions` and the committer's
     unwaited failures (`Hooks::reporting_to` → `agora/commit`);
   - the nested-ignore detection;
   - the reader half of killed mid-append: a torn last line is skipped, and damage in the
     middle is counted.

   Correct SDD-RUST §4.5's "today's position-based cursor", since today's reader pages by
   `seq`, and extend §10.
4. **Close it** the way R1.4a closed:
   - the owed tests;
   - a design-fidelity review and a refutation pass, both read-only, with every finding
     fixed or recorded;
   - `cargo xtask mutants` over every crate whose sources changed, with no TIMEOUT and an
     honest control;
   - Linux in WSL, plain **and** instrumented (`cargo llvm-cov --workspace --no-report`),
     at least 10 runs each, on a copy hashed against its commit;
   - push, then CI green on all three OSes, cited by run ID. No new crate, so no floor to
     seed, but no floor may be lowered, and a raise goes only through
     `cargo xtask coverage ratchet`;
   - the PROGRESS tick, the README marker (`R1.4b`) and paragraph, the DECISIONS-LOG
     implementation entry, and `docs/implementations/<date>-r1-4b-book-of-record.md`.

## Then

- **R1's exit** (RUST-PLAN): FR-4.1, FR-4.2, FR-4.4 and NFR-7 by test; nothing lost or
  torn on reopen after a kill mid-append or mid-commit; a mutation round with a control;
  every test that runs git does so hermetically. Close it with `/milestone-review` and
  record the verdict in PROGRESS.
- **R2** starts only after the Architect approves R2's package list. RUST-PLAN §9 owes
  one R2 question: whether `portable-pty`'s process-tree kill suffices on Windows or a
  job object is needed, which also closes R1.3's deadline-kill residual. Every later phase
  is the same. At parity (R14) the TypeScript tree is deleted, and whether M9 may start is
  asked.

## Settled: cite these, do not re-ask

- **2026-10-19**
  - ADR-0041: the rewrite, window included.
  - The Architect approved RUST-PLAN, crate Set A (RUST-CRATES), GYM-013's thresholds and the SRS wording.
  - Each package gets one branch and one PR, stacked.
- **2026-10-20, R1.0 to R1.3**
  - SDD-RUST §2 and §4 are approved.
  - A Rust home carries `home.json`.
  - Changed formats are `schemaVersion` 2.
  - The committer holds its own OS lock.
  - The Agora's git is hermetic.
  - The git-kill defect is recorded now and fixed in R2.
  - A detached worktree whose commit nothing holds is refused.
- **2026-10-20, R1.4**
  - The split into R1.4a and R1.4b.
  - Done-task retention past 9,000.
  - Severity waits for R4.
  - `DIAGNOSIS.md` moves to R7.
- **2026-10-25, R1.4a**
  - SDD-RUST §10 is approved.
  - The archive takes one line per run, through `append_line`.
  - The `.gitignore` is rendered at every boot and narrowed.
  - `PROTOCOL.md` is corrected at R5.
  - `Store` re-reads on every call.
  - The ledger's operations land with their callers.
  - The committer detects nested ignores.
  - Two R1.3 tests were fixed on R1.4a.
  - Open finishes a killed `git init`.
- **2026-10-25, R1.4b:** the four answers in step 1.

## Waiting on the Architect

- **Open the TypeScript fix PR.** Its body was drafted outside the tree. If that draft is
  gone, redraft it from `docs/implementations/2026-10-20-worktree-untracked-files.md`,
  which is on that branch only.

  ```bash
  gh pr create --base main --head fix/worktree-untracked-files --title "fix(worktrees): never remove untracked work a repo's config hides" --body-file <the body file>
  ```
- **Review #104.** Each stacked branch then gets its PR in turn, R1.4a after R1.3. Each
  one hits the DECISIONS-LOG append-against-append conflict; resolve it main-first.
- **Clean up the old worktree** `ephesus-rust-rewrite-6ad89e`. It is detached at
  `e4f85d4` and still holds a stale, uncommitted SDD-RUST draft and a 258 MB `target/`
  inside OneDrive. The session may not touch it:

  ```bash
  git -C "C:/Users/senso/OneDrive/Masaüstü/ephesus/.claude/worktrees/ephesus-rust-rewrite-6ad89e" checkout -- docs/sdd/SDD-RUST.md
  ```

  ```bash
  cargo clean --target-dir "C:/Users/senso/OneDrive/Masaüstü/ephesus/.claude/worktrees/ephesus-rust-rewrite-6ad89e/target"
  ```

## The prompt

```text
/goal Work as the Ephesus senior software engineer on the Rust rewrite (ADR-0041). If this worktree lacks feature/r1-4b-book-of-record, ask the Architect how to move it before any edit. First read .claude/skills/ephesus-engineer/SKILL.md, docs/RUST-GOAL.md, docs/RUST-PLAN.md, docs/PROGRESS.md's Resume here block, the DECISIONS-LOG entries from 2026-10-19 on, docs/sdd/SDD-RUST.md and the open PRs. Then resume at RUST-GOAL.md's first owed R1.4b step, finish R1.4b, close R1 with /milestone-review, and carry on through RUST-PLAN's packages in order: rebuild each part in Rust, verify it, integrate it, test it, then move to the next. Before a package's code, ask the Architect through AskUserQuestion every choice its documents leave open; a choice already in docs/adr/ or DECISIONS-LOG is cited, not re-asked. Consult the advisor before each package's approach and before calling it done. A package closes with: its owed tests; an adversarial review whose findings are fixed or recorded; a mutation round with no TIMEOUT whose control missed every viable mutant; its branch pushed; CI green on all three OSes, cited by run ID; a new crate's coverage floor seeded from three CI attempts; its PROGRESS tick, DECISIONS-LOG entry and docs/implementations record. Before a package of a phase whose package list the Architect has not approved, ask for that approval. At parity (R14), ask whether M9 may start.
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
* No result is claimed for a platform or tree not run in this session; a CI result is cited with its run ID and commit, as tier B. A race fix is measured plain and instrumented.
* Every build, cargo xtask included, uses a CARGO_TARGET_DIR outside OneDrive, and no source changes during a mutation round.
```

## Its length

The CLI refuses a `/goal` condition longer than **4,000 characters**. It measures the
JavaScript `.length`, newlines included. Re-measure after any edit, and cut words rather
than rules:

```bash
node -e "const t=require('fs').readFileSync('docs/RUST-GOAL.md','utf8');const b=t.split('\`\`\`text\n')[1].split('\n\`\`\`')[0];console.log(b.replace(/^\/goal /,'').trim().length)"
```

## On this machine

- **`CARGO_TARGET_DIR`.** Export `CARGO_TARGET_DIR="$LOCALAPPDATA/ephesus-cargo-target"`
  in **every** shell call, `cargo xtask mutants` included. The alias builds xtask first,
  with whatever the shell has, and only then unsets the variable for cargo-mutants. On
  2026-10-25 a round run without it left a 258 MB `target/` inside OneDrive.
- **Free RAM.** Check it before a mutation round, and run rounds and WSL builds one at a
  time. A slower round can TIMEOUT, and a TIMEOUT voids it.
- **WSL2 (Ubuntu, rustup 1.96.0, git 2.53, uid 1000).** Work through script files, run
  as `MSYS_NO_PATHCONV=1 wsl.exe -d Ubuntu -- bash /mnt/c/.../script.sh`. Inline `$`
  variables expand in WSL's outer shell, and a removal inside a `-c` script is refused,
  so stage each copy into a fresh, new directory name. Build the copy from
  `git ls-files -co --exclude-standard` with `tar --force-local`. Give it a target
  directory in the WSL home, and check it file for file against `git ls-tree` before
  citing it.
- **Antivirus.** Defender's real-time protection is on (checked 2026-10-25). Name it with
  any timing.
- **The name `goal`** is shared: the repository has a project skill called `goal` (the
  milestone runner), and the CLI has its own `/goal`. The first time you paste the
  prompt, confirm the CLI answers that a goal is set.

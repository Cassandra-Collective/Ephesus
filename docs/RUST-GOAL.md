# The Rust rewrite goal prompt — a handover

Paste the block under [The prompt](#the-prompt), whole, as one `/goal` command, in a
session whose working tree has the branch named below checked out: on 2026-10-25,
`feature/r1-4b-book-of-record` (R1's tip), in the worktree `sharp-wescoff-0dc72f`. The
app's guard refuses edits in any worktree other than the session's own. So a session that
starts somewhere else first asks the Architect how to move the branch, as the 2026-10-25
sessions did: detach the old worktree, then check the branch out in the new one
(DECISIONS-LOG 2026-10-25, *R1.4a — Architect decisions*, item 4, and *R1.4b — Architect
decisions, second round*, item 8).

This is a **handover**, like [`M9-GOAL.md`](./M9-GOAL.md): it says where the rewrite
stands, what is already decided, and what comes next. The block is the **condition** the
goal evaluator holds the session to. It carries the Architect's direction of 2026-10-19
(ADR-0041): rebuild every part of the system in Rust, verify that it works, integrate and
test it, then move to the next part, and when everything built so far runs in Rust,
continue with the plans that follow. It replaces nothing. The standing prompt for any
session, `docs/GOAL.md`, is on the pushed and unmerged branch `docs/standing-goal-prompt`.

*Written 2026-10-20 at R1.4a's pause; rewritten 2026-10-25 when R1.4a closed, and again
the same day when R1 closed.*

---

## Where the rewrite stands

| Package | State | Where |
|---|---|---|
| R0.1 workspace skeleton | Merged | #102 |
| R0.2 the gate chain | Merged | #104 (`9aa30bd`, 2026-10-25) |
| R0.3, R1.0, R1.1, R1.2, R1.3, R1.4a | Done, pushed, stacked on R0.2; each gets its PR as the one below it merges, R0.3's next | `feature/r0-3-standards-learn-rust` … `feature/r1-4a-agora-store` @ `3fe7490` |
| R1.4b `eph-agora`, the book of record | **Done and ticked**, pushed. CI run 40206703962 on `237aa31` and 40207561007 on `512cf21` are green on every job | `feature/r1-4b-book-of-record`, cut from `3fe7490` |
| **R1** the home and the book of record | **DONE** (2026-10-25), its verdict in PROGRESS | the same branch |

R1.4b's close is recorded in PROGRESS (its tick, its evidence and R1's verdict), in
[`2026-10-25-r1-4b-book-of-record.md`](./implementations/2026-10-25-r1-4b-book-of-record.md),
and in seven DECISIONS-LOG entries of 2026-10-25. Its reviews changed earlier packages,
each on the Architect's answer:
- **Two R1.3 defects.** One agent's `git init` in its own directory stopped every Agora
  commit, and one with a commit hid its work as a gitlink. The committer now keeps a
  nested repository out through its own block of `.git/info/exclude` and names it.
- **An R1.4a defect.** The rendered `.gitignore` hid any directory named like a
  `write_atomic` temporary, with everything in it; it now re-includes such directories.

## Next: R2, the terminal plane and the engine seam

1. **Ask before any R2 code, through AskUserQuestion.** R2 starts only once the Architect
   approves its package list. RUST-PLAN §5 lists R2.1 `eph-pty` (spawn, write, resize,
   interrupt, kill; PATH resolution; the redaction filter), R2.2 `eph-engines` (the
   adapter trait, the `claude` adapter, `codex` and `gemini` as conformance subjects) and
   R2.3 the agent manager. RUST-PLAN §9 owes one R2 question, asked with it: whether
   `portable-pty`'s process-tree kill suffices on Windows or a job object (an FFI boundary
   crate, Set B, its own memo) is needed, which also closes R1.3's deadline-kill residual.
   Then each package's own open choices, before its code.
2. **R2.1's branch** is `feature/r2-1-eph-pty`, cut from `feature/r1-4b-book-of-record`'s
   tip. A new crate's first push is red at the coverage step until its floor is seeded
   from three CI attempts with `cargo xtask coverage ratchet`.
3. **Carried into R2:** GYM-014's metric 3 is measured at R2's close (no `process:`
   exemption in `eph-agora` outside test-only code); `.git/` is outside an agent's grant
   and R2.3's grants keep an agent out of it; a symbolic link is committed as the link,
   for R2.3 to decide; the prompt-literal rule arrives with R2's first prompt (R0.2's
   decision); `BOOT_CHECKED`'s fourteen causes are proved to clear end to end for two
   only, so R2's boot work should cover the rest.
4. **Each package closes** the way R1.4b did: its owed tests; a design-fidelity review and
   a refutation pass, both read-only; `cargo xtask mutants` over every crate whose sources
   changed, with no TIMEOUT and an honest control; 10 plain and 10 instrumented runs on
   win32 and on WSL2 Linux, on a copy hashed against its commit; push and CI green on all
   three OSes, cited by run ID; its PROGRESS tick, README marker and paragraph,
   DECISIONS-LOG implementation entry and implementation record.

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
- **2026-10-25, R1.4b** (six entries)
  - A channel per subscriber; a byte-offset cursor; one lock for the writer.
  - The ignore check rides the batch's own status; the harness's two shapes are matched.
  - The Agora's git reads no attributes from the tree; `.git/` is outside an agent's grant.
  - The archive reader is built; every `agora/commit` cause clears at a landing.
  - `agora/log-torn` at boot; `agora/log-size` for a slow read.
  - A kill mid-append is proved by plant and by real kill; `eph-agora` may claim `process:` for its kill tests (GYM-014).
  - A directory named like a temporary file is re-included.
  - A nested repository is kept out and named.
  - A boot clears the boot causes it no longer finds.
  - The channel is the one writer of degradation lines; links are recorded, not named.
  - R1.4b's SDD-RUST edits are approved; one equivalent mutant is excluded; the branch is pushed for CI.

## Waiting on the Architect

- **Open R0.3's PR**, next in the stack now that #104 has merged. Each stacked branch then
  gets its PR in turn. Each one hits the DECISIONS-LOG append-against-append conflict;
  resolve it main-first.

  ```bash
  gh pr create --base main --head feature/r0-3-standards-learn-rust --title "R0.3: the standards learn Rust" --body-file <the body file>
  ```
- **Open the TypeScript fix PR.** Its body was drafted outside the tree. If that draft is
  gone, redraft it from `docs/implementations/2026-10-20-worktree-untracked-files.md`,
  which is on that branch only.

  ```bash
  gh pr create --base main --head fix/worktree-untracked-files --title "fix(worktrees): never remove untracked work a repo's config hides" --body-file <the body file>
  ```
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
/goal Work as the Ephesus senior software engineer on the Rust rewrite (ADR-0041). If this worktree lacks the branch docs/RUST-GOAL.md names, ask the Architect how to move it before any edit. First read .claude/skills/ephesus-engineer/SKILL.md, docs/RUST-GOAL.md, docs/RUST-PLAN.md, docs/PROGRESS.md's Resume here block, the DECISIONS-LOG entries from 2026-10-19 on, docs/sdd/SDD-RUST.md and the open PRs. Then resume at RUST-GOAL.md's next step, close each phase with /milestone-review, and carry on through RUST-PLAN's packages in order: rebuild each part in Rust, verify it, integrate it, test it, then move to the next. Before a package's code, ask the Architect through AskUserQuestion every choice its documents leave open; a choice already in docs/adr/ or DECISIONS-LOG is cited, not re-asked. Consult the advisor before each package's approach and before calling it done. A package closes with: its owed tests; an adversarial review whose findings are fixed or recorded; a mutation round with no TIMEOUT whose control missed every viable mutant; its branch pushed; CI green on all three OSes, cited by run ID; a new crate's coverage floor seeded from three CI attempts; its PROGRESS tick, DECISIONS-LOG entry and docs/implementations record. Before a package of a phase whose package list the Architect has not approved, ask for that approval. At parity (R14), ask whether M9 may start.
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
  2026-10-25 a round run without it left a 258 MB `target/` inside OneDrive. A scratch
  copy of the workspace gets its own target directory: one built into the shared one left
  its `xtask.exe` in `debug/`, and the worktree's CLI tests then ran it (memory: *a shared
  target dir leaks another checkout's binary*).
- **Free RAM.** Check it before a mutation round, and run rounds and WSL builds one at a
  time. A slower round can TIMEOUT, and a TIMEOUT voids it. A test that does thousands of
  synced appends is a TIMEOUT waiting for the mutant that stops a counter.
- **WSL2 (Ubuntu, rustup 1.96.0, git 2.53, uid 1000).** Work through script files, run
  as `MSYS_NO_PATHCONV=1 wsl.exe -d Ubuntu -- bash /mnt/c/.../script.sh`. Inline `$`
  variables expand in WSL's outer shell, and a removal inside a `-c` script is refused,
  so stage each copy into a fresh, new directory name. Build the copy from a committed
  tree with `git archive`, give it a target directory in the WSL home, and check it file
  for file against `git ls-tree` (with `git hash-object --no-filters`) before citing it.
- **Antivirus.** Defender's real-time protection is on (checked 2026-10-25). Name it with
  any timing.
- **The name `goal`** is shared: the repository has a project skill called `goal` (the
  milestone runner), and the CLI has its own `/goal`. The first time you paste the
  prompt, confirm the CLI answers that a goal is set.

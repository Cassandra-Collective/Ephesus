# The Rust rewrite goal prompt — a handover

Paste the block under [The prompt](#the-prompt), whole, as one `/goal` command, in a
session whose working tree has the branch named below checked out: on 2026-11-23,
`feature/r2-2-eph-engines`. That branch does not exist yet. The stack each package was
cut from has merged, so R2.2's branch is cut from `main` at `45dcff9` or later, in the
session's own worktree (DECISIONS-LOG 2026-11-23, item 1). That answer is cited, not
re-asked. The app's guard refuses edits in any worktree other than the session's own. So
a session that needs a branch checked out somewhere else first asks the Architect how to
move it, as the 2026-10-25 sessions did: detach the old worktree, then check the branch
out in the new one (DECISIONS-LOG 2026-10-25, *R1.4a — Architect decisions*, item 4, and
*R1.4b — Architect decisions, second round*, item 8).

This is a **handover**, like [`M9-GOAL.md`](./M9-GOAL.md): it says where the rewrite
stands, what is already decided, and what comes next. The block is the **condition** the
goal evaluator holds the session to. It carries the Architect's direction of 2026-10-19
(ADR-0041): rebuild every part of the system in Rust, verify that it works, integrate and
test it, then move to the next part, and when everything built so far runs in Rust,
continue with the plans that follow. It replaces nothing. The standing prompt for any
session, `docs/GOAL.md`, is on the pushed and unmerged branch `docs/standing-goal-prompt`.

*Written 2026-10-20 at R1.4a's pause; rewritten 2026-10-25 when R1.4a closed, again the
same day when R1 closed, and updated when the Architect approved R2's list and paused
the session before R2.1's probe. Updated again on 2026-11-23, when every package
through R2.1 had merged to `main`.*

---

## Where the rewrite stands

| Package | State | Where |
|---|---|---|
| R0.1, R0.2, R0.3 | Merged | #102, #104 (`9aa30bd`), #105 |
| R1.0 to R1.4b | Merged. **R1 is DONE** (2026-10-25), its verdict in PROGRESS | #106, #108 to #112 |
| R2.1 `eph-pty` | **Done, ticked and merged** (2026-11-23). CI run 45196366436 on `45dcff9` is green on every job | #107 (`45dcff9`), the last to merge |
| **R2.2 `eph-engines`** | **Next.** Not started: no branch and no code | — |
| R2.3 the agent manager | Not started | — |

R2.1's close is recorded in PROGRESS (its tick and the resume block), in the
[probe](./implementations/2026-11-16-r2-1-pty-tree-probe.md) and
[proof](./implementations/2026-11-20-r2-1-guard-proof.md) records, in
[ADR-0042](./adr/ADR-0042-owned-process-group-shutdown.md) and
[ADR-0043](./adr/ADR-0043-preserve-the-terminal-spawn-plan.md), and in the
DECISIONS-LOG entries of 2026-11-16 to 2026-11-23. Beyond its own crate:
- **A new FFI crate, `eph-process`,** holds every native lifecycle call: Windows Job
  Objects at creation (`windows-sys` 0.61) and owned POSIX process groups (`libc` 0.2).
- **R1.3's deadline-kill residual is answered.** Git runs under the same owned shutdown,
  so a git past its deadline gets a bounded cleanup of its owned group.
- **The Agora recovers two known ref locks at open.** `HEAD.lock` and
  `refs/heads/main.lock` are removed under the committer lock and reported as
  `agora/ref-lock`.
- **Two new floors:** `eph-process` 94.88% and `eph-pty` 94.94%, seeded from CI run
  45122195829, whose three attempts reported identical line counts on every platform.

## Next: R2.2, the engine seam

1. **Already answered, cite them:**
   - R2's package list (DECISIONS-LOG 2026-10-25, *R2 — Architect decisions*): R2.2
     `eph-engines` is the adapter trait, the `claude` adapter, and `codex` and `gemini`
     as conformance subjects. Each package's open choices are asked before its code.
   - R2.2's branch is `feature/r2-2-eph-engines`, cut from `main`. SDD-RUST §3 is
     approved whole once R2.2 has written its engine-adapter half (DECISIONS-LOG
     2026-11-23, items 1 and 3).
   - An engine adapter may not match on output this repository has never seen
     (DECISIONS-LOG 2026-09-04).
2. **R2.2's first step is the design.** SDD-RUST's table gives §3's engine-adapter half
   to R2.2. Read the TypeScript reference before designing anything (RUST-PLAN §6):
   `src/main/engines/types.ts` (the adapter surface), `claude.ts`, `codex.ts`,
   `gemini.ts`, `engine-home.ts`, `settings-install.ts`, `spawn-env.ts`, `tool-grants.ts`
   and `index.ts`, with their tests. RUST-PLAN §5 names what the `claude` adapter carries:
   the spawn plan, the per-agent config directory (ADR-0026), workspace trust (ADR-0021,
   ADR-0025), autonomy (ADR-0031), attribution suppression (FR-1.8), no self-upgrade
   (ADR-0028) and prompts declared in advance (ADR-0035). `codex` and `gemini` are
   unregistered conformance subjects (ADR-0024), and the conformance table is checked in
   both directions (FR-1.10). Record the installed CLIs' real output as fixtures before
   writing any matcher. Then the whole of §3 goes to the Architect for one approval.
3. **Then R2.2's open choices, before its code.** The 2026-11-23 audit named three
   candidates, none of them decided:
   - which CLI versions the fixtures pin;
   - whether conformance needs the `codex` and `gemini` binaries installed, or recorded
     output is enough;
   - how `eph-engines`' version probes start their process: RUST-PLAN §3.2 allows them,
     and ADR-0042 confines native lifecycle calls to `eph-process`.

   Then consult the advisor on the approach. A new crate's first push is red at the
   coverage step until its floor is seeded from three CI attempts with `cargo xtask
   coverage ratchet`.
4. **R2.3, the agent manager, follows R2.2.** Its reference is `src/main/agents.ts` and
   `src/main/respawn.ts`. One open choice is already visible: RUST-PLAN §3.1 names no
   crate for it, and `eph-core` holds no domain logic.
5. **R2's exit** (RUST-PLAN §5): a real `claude` runs in a PTY under a headless test
   binary, with its own config directory and suppressed attribution; the conformance
   table is green for every subject in both directions; FR-1.1 to FR-1.10. The live run
   needs an installed, signed-in CLI, so it runs on the Architect's machine (tier A), not
   in CI. Parts of FR-1.3, FR-1.4 and FR-1.9 are built by R4, R3 and R6. R1's verdict
   met the same problem with a column naming what later phases owe.
6. **Carried into R2:** GYM-014's metric 3 is measured at R2's close (no `process:`
   exemption in `eph-agora` outside test-only code); `.git/` is outside an agent's grant
   and R2.3's grants keep an agent out of it; a symbolic link is committed as the link,
   for R2.3 to decide; the prompt-literal rule arrives with R2's first prompt (R0.2's
   decision); `BOOT_CHECKED`'s fourteen causes are proved to clear end to end for two
   only, so R2's boot work should cover the rest. R2.1 adds three residuals:
   - on POSIX, descendants that move to another process group are outside the shutdown
     guarantee (ADR-0042);
   - on macOS, a dropped session's cleanup worker has no caller to report an
     unconfirmed shutdown to (SDD-RUST §3);
   - the excluded removal of `Attributes::drop` rests on code review, not a runtime proof.
7. **Each package closes** the way R1.4b did: its owed tests; a design-fidelity review and
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
- **2026-11-16 to 2026-11-23, R2.1**
  - ADR-0042: Windows Job Objects at creation and owned POSIX process groups, in `eph-process`, with `windows-sys` 0.61 and `libc` 0.2.
  - ADR-0043: the exact spawn plan is kept through native `openpty`; launch values stay in the redactor until exit; a Windows batch shim runs through a checked interpreter in the job.
  - Each native backend is required on the CI platforms that compile it.
  - `HEAD.lock` and `refs/heads/main.lock` are recovered at open and reported as `agora/ref-lock`.
  - Equivalent flag mutants and the removal of `Attributes::drop` are excluded; the Darwin guards are proven on macOS by a manually dispatched workflow.
  - Resize is proven through native terminal-size observation behind a `test-support` feature.
  - On macOS, unconfirmed PTY shutdown is reported after 500 ms and ownership is kept.
- **2026-11-23, the record fix**
  - R2.2 is cut from `main`.
  - `toleranceReason` was corrected by hand, once; otherwise the floors file changes only through the ratchet.
  - SDD-RUST §3 is approved whole at R2.2.
  - The TypeScript fix PR stays owed.

## Waiting on the Architect

- **Pull the main checkout.** On 2026-11-23 the repository root's checkout was at
  `8a3a3a7` (2026-10-11), 134 commits behind `origin/main`. R0's carried debt says the
  skills' new text reaches a session only once that checkout pulls. Check it is clean
  first:

  ```bash
  git -C "C:/Users/senso/OneDrive/Masaüstü/Cassandra-Collective/ephesus" status -sb
  ```

  ```bash
  git -C "C:/Users/senso/OneDrive/Masaüstü/Cassandra-Collective/ephesus" pull --ff-only
  ```
- **Open the TypeScript fix PR** (kept owed, DECISIONS-LOG 2026-11-23, item 4).
  `fix/worktree-untracked-files` is two commits ahead of a `main` that has moved since
  2026-10-20, so trial-merge it before opening. Its body was drafted outside the tree. If that draft is
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

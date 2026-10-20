# A worktree's untracked work is never removed, whatever the target repository's config says

**Date:** 2026-10-20 · **Branch:** `fix/worktree-untracked-files` · **Requirement:** SRS
FR-1.5 (nothing at a worktree path is deleted to make room; a worktree with uncommitted
changes is kept) · **Decision:** DECISIONS-LOG 2026-10-20, *TypeScript worktree
untracked-files fix* · **Found by:** the Rust rewrite's R1.3 refutation pass

## 1. Problem / motivation

`Worktrees.remove` promises to keep a worktree that holds uncommitted work and to name
the changes. It decided "clean" from a bare `git status --porcelain`, then ran
`git worktree remove`. Both read the target repository's own configuration. A target
whose `.git/config`, or the user's global config, sets `status.showUntrackedFiles=no`
hides untracked files from both. Measured on git 2.53.0.windows.2:

1. An agent's new, uncommitted `wip.md` in its worktree.
2. `status --porcelain` printed nothing.
3. `git worktree remove` exited 0 and deleted the directory, `wip.md` with it.
4. With `-c status.showUntrackedFiles=all`, git refused: *"contains modified or untracked
   files, use --force to delete it"*.

This build runs the Architect's company until the Rust rewrite reaches parity (R14), so
the TypeScript tree is changed despite its freeze. The Architect allowed this one
exception because it is data loss.

## 2. What changed

| File | Change |
|---|---|
| `src/main/git.ts` | `state()` runs `status --porcelain --untracked-files=all`; `remove()` runs `git -c status.showUntrackedFiles=all worktree remove`. |
| `test/main/worktrees.test.ts` | Two regression tests. The three runner wrappers find `worktree remove` by its subcommand, not by position (`removedBy`). |
| `docs/DECISIONS-LOG.md` | The decision. |

## 3. Implementation approach

There are two overrides, one per check, because either check alone can be outrun:

- **`state()` asks for every untracked file** with the `--untracked-files=all` argument,
  so the harness's own refusal sees what the agent wrote.
- **`git worktree remove` gets `-c status.showUntrackedFiles=all`.** Git hands command-line
  configuration to the `git status` it runs for its own clean check. So a file written
  after `state()` looked, the race between the two calls, is still refused rather than
  deleted.

Nothing else about the user's configuration changes for a target repository. The two
overrides are a status display setting, applied to the two calls that decide whether a
directory may be deleted.

## 4. Mathematical / statistical details

None: a structural change.

## 5. Design decisions

- **Override the setting rather than refuse a repository that sets it.** The setting is a
  legitimate display choice for a human, and a refusal would retire the agent. Asking
  git for every untracked file costs one argument.
- **Both checks, not one.** `state()` alone leaves the race. `remove`'s override alone
  would leave `state()` reporting clean, and a caller that reads `state()` to decide
  (the agent's exit unwind) would be told so.
- **Ignored files are unchanged:** `git worktree remove` still deletes them with the
  worktree, as before. The Rust rewrite records that boundary of FR-1.5 explicitly.

## 6. Verification

Win32, git 2.53.0.windows.2, node_modules linked from the main checkout after all 428
installed packages were checked against `package-lock.json`.

- `npx vitest run test/main/worktrees.test.ts test/main/agent-worktree.test.ts`: 46
  passed.
- **Differential.** With `main`'s `src/main/git.ts` and the new tests, both new tests
  fail and the other 33 pass:
  - *"REFUSES a dirty one even when the target repository hides untracked files"*
  - *"keeps the work when git's own clean check is the last one left"*

  Each new test pins one override. The first asserts `state()`'s answer. The second
  makes every `status` read clean, so only `remove`'s own override stands between the
  file and its deletion.
- `npm run typecheck`, `npm run lint`, `node scripts/check-invariants.cjs` and
  `node scripts/check-readme-current.cjs` are green.

## 7. Related docs

- SRS FR-1.5; ADR-0004 (why worktrees live beside the single committer).
- `docs/implementations/2026-09-06-an-emptied-worktree-path-retired-the-agent.md`: the
  vacancy rule this module already follows.

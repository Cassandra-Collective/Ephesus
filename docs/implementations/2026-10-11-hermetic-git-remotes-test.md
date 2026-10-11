# `repo-remotes.test.ts` reads git without the machine's git config

**Date:** 2026-10-11 · **Branch:** `fix/hermetic-git-remotes-test` from `main`
`75ec37c`

---

The "readRemotes, against git itself" block in `test/main/repo-remotes.test.ts` now
runs git with the system file, the global file and the environment's config switched
off, so it tests the parser against git's own output rather than against whatever
the host has configured. Production is unchanged.

## 1. Problem / motivation

On 2026-10-11, in a cloud container, "reads a separate push URL as its own line"
failed at `expect(read.remotes).toHaveLength(2)`: one remote where two were expected.
The container injected a URL rewrite into every git process through the environment:

```text
GIT_CONFIG_COUNT=1
GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf
GIT_CONFIG_VALUE_0=git@github.com:
```

`git remote -v` reports URLs after `insteadOf` rewriting, and that applies to push
URLs too. The test sets the fetch URL to `https://github.com/owner/app.git` and the
push URL to `git@github.com:owner/app.git`. Under the rewrite git prints the push URL
as `https://github.com/owner/app.git`, the same as the fetch line, and `readRemotes`
merges the two identical pairs. That is what it should do. **The production code is
not wrong. The test depended on the host's git configuration.**

The same failure reproduced on Windows with the same three variables (Windows 11,
git 2.53.0.windows.2, Node v20.16.0, vitest 4.1.11): 1 failed, 7 passed. Every other
source git reads configuration from caused it too (§6).

## 2. What changed

| File | Change |
|---|---|
| `test/main/repo-remotes.test.ts` | The "against git itself" block stubs `process.env` in `beforeAll` (config-value variables deleted, `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`) and restores it in `afterAll`, with a comment giving the cause. |
| `docs/implementations/2026-10-11-hermetic-git-remotes-test.md` | This note. |
| `docs/DECISIONS-LOG.md` | One appended entry. |

## 3. Implementation approach

`ExecGitRunner.run` (`src/main/git.ts`) calls `execFile` without an `env` option, and
so does the test's `git()` helper (`execFileSync`). Both children therefore inherit
the vitest worker's `process.env` as it is at spawn time. Stubbing `process.env` once
covers both spawners, with no change to production code.

While the block runs:

- **Every variable that carries config values is deleted:** `GIT_CONFIG_COUNT`,
  `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`, and `GIT_CONFIG_PARAMETERS`. The last
  is how a parent `git -c …` passes its flags on to the processes it starts, for
  example a test run launched from a hook or a `rebase --exec`. The names are found
  by enumerating `process.env` against
  `^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+|PARAMETERS)$`, and each is removed with
  `vi.stubEnv(name, undefined)`.
- **`GIT_CONFIG_NOSYSTEM=1`** skips the system file. It wins over
  `GIT_CONFIG_SYSTEM`, which is checked in §6.
- **`GIT_CONFIG_GLOBAL=/dev/null`** replaces both `~/.gitconfig` and
  `$XDG_CONFIG_HOME/git/config`. git's documentation names `/dev/null` for this
  variable. It needs git 2.32 or later.

`afterAll` calls `vi.unstubAllEnvs()`. Vitest's default pool is `forks` with
`isolate` on, and `vitest.config.mts` sets neither, so each test file runs in its own
process and the stubs cannot reach another file. The restore keeps them out of the
second block in this file too, which uses an injected runner and never spawns git.

What git still reads: the temp repository's own `.git/config`, and the `-c` flags each
caller passes (`ExecGitRunner`'s `IDENTITY_ARGS`, the helper's `user.name` and
`user.email`). Those are part of what is under test.

## 4. Mathematical / statistical details

None. This changes a test's environment and involves no formula or numeric
algorithm.

## 5. Design decisions

1. **Stub `process.env` in the test: chosen.** No production change, and the runner
   under test is the real `ExecGitRunner` with its real arguments.
2. **An `env` parameter on `ExecGitRunner`: rejected.** It would add a test-only API
   to the one module allowed to run git (ADR-0004). Production has no caller that
   would pass it.
3. **Isolate in production as well: rejected.** For a real target repository the
   user's config is meant to apply. If someone rewrites `git@github.com:` to
   `https://`, that is how their checkout talks to GitHub, and `readRemotes` should
   report what git reports. DECISIONS-LOG 2026-08-26 isolates the *Agora's* commits
   from the machine's config. That is a different repository with a different owner,
   and this change does not extend that rule to target repositories.
4. **This is not the "config only in tests" option that pacing-wakes rejected.**
   `docs/implementations/2026-10-02-pacing-wakes-teardown.md` §5 option 5 rejected
   *adding* a git setting in tests, because the suite would then stop exercising
   production's git. This change adds no setting. It removes the host's settings,
   and production's own `-c` flags still reach git unchanged.
5. **`/dev/null`, not `os.devNull` or an empty temp file.** Measured on Git for
   Windows 2.53.0: `GIT_CONFIG_GLOBAL=/dev/null` works, because Git for Windows maps
   that name itself. Node's `os.devNull` is `\\.\nul` on Windows, and with it git
   exits 128 with "unable to access '\\.\nul': Invalid argument". An empty file would
   need a temp directory, and with it the temp-hygiene rules, for no gain.
6. **Scope: this file only (Architect's choice, 2026-10-11).** The same exposure
   exists elsewhere, and those cases are recorded as follow-ups below rather than
   fixed here:
   - `test/scenarios/s-profile.test.ts` reads remotes through real git with
     `https://github.com/…` URLs. It would fail under an `insteadOf` that matches
     `https://github.com/` (for example, a proxy rewrite), but not under the one
     observed.
   - `test/main/worktrees.test.ts` (3 `commit` calls) and
     `test/main/agent-worktree.test.ts` (1) commit through `execFileSync` without
     `commit.gpgsign=false` or a neutered `core.hooksPath`. A global
     `commit.gpgsign=true` or hooks path reaches their fixture commits.
   - No guard requires the isolation. A future real-git test can be written without
     it, and that would only show up on a machine whose config changes the result.
     Adding a guard would change what a CI gate refuses, which needs a Gymnasium row
     first (ENGINEERING-STANDARDS §3).

## 6. Verification

Run on Windows 11 (10.0.26300), git 2.53.0.windows.2, Node v20.16.0, vitest 4.1.11,
from a session worktree whose `node_modules` was a junction to the main checkout's
(same lockfile, 328 packages, 0 version mismatches).

**Each config source, before and after.** Five ways to inject the same rewrite,
`url.https://github.com/.insteadOf = git@github.com:`. "Before" is the file as it is
on `main`; "after" is this change.

| Injected through | Before | After |
|---|---|---|
| nothing | 8 passed | 8 passed |
| `GIT_CONFIG_COUNT=1`, `GIT_CONFIG_KEY_0`, `GIT_CONFIG_VALUE_0` (the observed case) | 1 failed, 7 passed | 8 passed |
| `GIT_CONFIG_PARAMETERS="'url.https://github.com/.insteadOf'='git@github.com:'"` | 1 failed, 7 passed | 8 passed |
| `GIT_CONFIG_GLOBAL=<file holding the rewrite>` | 1 failed, 7 passed | 8 passed |
| `GIT_CONFIG_SYSTEM=<the same file>` | 1 failed, 7 passed | 8 passed |
| `XDG_CONFIG_HOME=<dir>` with the rewrite in `git/config` | 1 failed, 7 passed | 8 passed |

In every "before" failure, the failing case was "reads a separate push URL as its own
line" at `repo-remotes.test.ts:77`. The observed case, before:

```text
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0='url.https://github.com/.insteadOf' \
  GIT_CONFIG_VALUE_0='git@github.com:' npx vitest run test/main/repo-remotes.test.ts
 × reads a separate push URL as its own line
AssertionError: expected [ { name: 'origin', …(1) } ] to have a length of 2 but got 1
 Tests  1 failed | 7 passed (8)
```

and after, the same command gives `Tests  8 passed (8)`. The "before" row for XDG
ran the file from `main` as a temporary copy, deleted afterwards. The other "before"
rows ran ahead of the edit.

**The same mechanism on Linux git.** Plain git, not the suite: git 2.53.0 in WSL2
Ubuntu, a temp repository with the test's two URLs.

```text
-- rewrite injected (GIT_CONFIG_COUNT/KEY_0/VALUE_0):
origin	https://github.com/owner/app.git (fetch)
origin	https://github.com/owner/app.git (push)
-- rewrite in a GIT_CONFIG_SYSTEM file + NOSYSTEM + GLOBAL=/dev/null:
origin	https://github.com/owner/app.git (fetch)
origin	git@github.com:owner/app.git (push)
-- the same file as GIT_CONFIG_SYSTEM without NOSYSTEM (control):
origin	https://github.com/owner/app.git (fetch)
origin	https://github.com/owner/app.git (push)
```

**`/dev/null` against `os.devNull` on Git for Windows:** with
`GIT_CONFIG_GLOBAL=/dev/null`, `git config --list --show-scope` lists no `system` or
`global` entries. With `GIT_CONFIG_GLOBAL=\\.\nul` (Node's `os.devNull`), git exits
128 with `fatal: unable to access '\\.\nul': Invalid argument`.

**Gates:**

```text
npm run typecheck                    exit 0
npm run lint                         exit 0  (All matched files use Prettier code style!)
node scripts/check-invariants.cjs    exit 0  (invariants ok … reachability 189/199)
node scripts/check-readme-current.cjs exit 0
npx vitest run test/temp-hygiene.test.ts test/scripts/check-invariants.test.ts \
  test/main/repo-remotes.test.ts test/scenarios/s-profile.test.ts
 Test Files  4 passed (4)
      Tests  294 passed (294)
```

**Full suite (`npx vitest run`), not green in this condition, and not because of
this change.** It ran in this OneDrive worktree, at default parallelism, with
0.7–2.8 GB free. Earlier gates ran outside OneDrive. There were three runs:

| Tree | Result |
|---|---|
| this change | exit 1: 1 failed, 4918 passed, 8 skipped, 239 files |
| this change, rerun | exit 1: the same |
| `main`'s `repo-remotes.test.ts` restored (`src`, `shims`, `scripts`, `test` identical to `75ec37c`) | exit 1: the same |

In all three, the one failure is `check-invariants.test.ts` › "the CLI, run as CI
runs it" › "exits 0 over this repository". Its `spawnSync` of
`scripts/check-invariants.cjs` hits `CHILD_TIMEOUT_MS` (20 s) with `ETIMEDOUT`, at
20 108–20 175 ms. Run alone, the checker exits 0 in 3 226 ms, and that test file
passes 20/20. The control run fails the same way without this change, so the
timeout is pre-existing in this condition. It is recorded here and not investigated
further.

**To reproduce:**

```bash
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0='url.https://github.com/.insteadOf' GIT_CONFIG_VALUE_0='git@github.com:' npx vitest run test/main/repo-remotes.test.ts
```

**Not proven:** a green full suite on this machine (see above), and the vitest run
on Linux, which is where the failure was first seen.
Only plain Linux git was run, above. CI's ubuntu runner will run the file, but
without any injected rewrite. A git older than 2.32 ignores `GIT_CONFIG_GLOBAL`, so
there a `~/.gitconfig` rewrite would still reach the test. Git 2.53 was the only
version run.

## 7. Related docs

- `test/main/repo-remotes.test.ts`: the comment above the block.
- `src/main/git.ts`: `ExecGitRunner`, `readRemotes`, `IDENTITY_ARGS`.
- `docs/DECISIONS-LOG.md`: 2026-08-26 (the Agora's isolation from the machine's
  config) and this change's entry.
- `docs/implementations/2026-10-02-pacing-wakes-teardown.md` §5: the rejected
  "config only in tests" option, distinguished in §5.4 above.
- `docs/TEST-STRATEGY.md` §2: real git in temp directories.

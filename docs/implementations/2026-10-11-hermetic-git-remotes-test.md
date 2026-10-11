# `repo-remotes.test.ts` reads git without the machine's git config

**Date:** 2026-10-11 · **Branch:** `fix/hermetic-git-remotes-test` from `main`
`75ec37c`

---

The "readRemotes, against git itself" block in `test/main/repo-remotes.test.ts` now
runs git with the system file, the global file and the environment's config switched
off, so it tests the parser against git's own output rather than against whatever
the host has configured. So that a regression cannot pass on a clean machine, each of
its cases first plays the hostile host that broke it, and a mutation round shows that
every line of the isolation is needed. Production is unchanged.

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
| `test/main/repo-remotes.test.ts` | Before each "against git itself" case, plays the hostile host through four routes, then isolates git from it (config-value variables deleted, matched without regard to case; `GIT_CONFIG_NOSYSTEM=1`; `GIT_CONFIG_GLOBAL=/dev/null`), and restores `process.env` after the case. A new block checks that each route alone rewrites the push URL. A comment gives the cause. |
| `test/mutation/repo-remotes-isolation.json` | The mutation round's spec (TEST-STRATEGY §10): a control and six mutants. |
| `docs/implementations/2026-10-11-hermetic-git-remotes-test.md` | This note. |
| `docs/DECISIONS-LOG.md` | Two appended entries. |

## 3. Implementation approach

`ExecGitRunner.run` (`src/main/git.ts`) calls `execFile` without an `env` option, and
so does the test's `git()` helper (`execFileSync`). Both children therefore inherit
the vitest worker's `process.env` as it is at spawn time. Stubbing `process.env` once
covers both spawners, with no change to production code.

### The isolation

`isolateGitFromTheHost()` does three things:

- **It deletes every variable that carries config values:** `GIT_CONFIG_COUNT`,
  `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`, and `GIT_CONFIG_PARAMETERS`. The last
  is how a parent `git -c …` passes its flags on to the processes it starts, for
  example a test run launched from a hook or a `rebase --exec`. The names are found
  by enumerating `process.env` against
  `/^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+|PARAMETERS)$/`, and each is removed with
  `vi.stubEnv(name, undefined)`. The match is case-sensitive on purpose (§5.8).
- **It sets `GIT_CONFIG_NOSYSTEM=1`,** which skips the system file. That wins over
  `GIT_CONFIG_SYSTEM`; see §6 and the round's `system-file-read` mutant.
- **It sets `GIT_CONFIG_GLOBAL=/dev/null`,** which replaces both `~/.gitconfig` and
  `$XDG_CONFIG_HOME/git/config`. git's documentation names `/dev/null` for this
  variable. It needs git 2.32 or later.

It runs in a `beforeEach`, and an `afterEach` calls `vi.unstubAllEnvs()`. A
`beforeAll` would also work today, but only because vitest's `unstubEnvs` option is
off. That option clears stubs before every case (vitest 4.1.11), so turning it on
would silently undo stubs made once, and every case would stay green on a clean host.
Vitest's default pool is `forks` with `isolate` on, and `vitest.config.mts` sets
neither, so each test file runs in its own process and the stubs cannot reach another
file.

### The hostile host

On a clean host, such as CI's runner or most laptops, the isolation has nothing to
undo. Deleting it, or any one line of it, would leave every case green there. So
before the isolation, each case plays the host that broke this file, through every
route git reads the same rewrite from (`HOSTILE_ROUTES`):

| Route | What it sets |
|---|---|
| `GIT_CONFIG_COUNT` | `GIT_CONFIG_COUNT=1`, `GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf`, `GIT_CONFIG_VALUE_0=git@github.com:` |
| `GIT_CONFIG_PARAMETERS` | `'url.https://github.com/.insteadOf'='git@github.com:'` |
| a global file | `GIT_CONFIG_GLOBAL=<file holding the rewrite>` |
| a system file | `GIT_CONFIG_SYSTEM=<the same file>`, with `GIT_CONFIG_NOSYSTEM` deleted |

The file sits in an `mkdtemp` directory that the file's module-level `afterEach`
removes with `removeTempDir`, as the temp-hygiene rules require. The system route
deletes `GIT_CONFIG_NOSYSTEM` because a host that had already set it would otherwise
make that route inert. `XDG_CONFIG_HOME` is not a separate route here: git does not
read it while `GIT_CONFIG_GLOBAL` is set, and the global route already covers that
scope.

Each route is enough on its own. A new block, "the hostile host, one route at a
time", checks that: it applies one route with no isolation and asserts that git
printed the push URL as a copy of the fetch URL. Without that block, a route git
stopped honouring, say on a different version, would make the guard decoration that
stays green.

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
   `docs/implementations/2026-10-02-pacing-wakes-teardown.md` §6 option 5 rejected
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
   - Only config is isolated. Variables that locate a repository (`GIT_DIR`,
     `GIT_WORK_TREE`, `GIT_COMMON_DIR`, …) would still reach git, for example in a
     suite started from a git hook. No hook in `.githooks/` runs the suite.
7. **A regression test that plays the hostile host (Architect's choice after review,
   2026-10-11).** Review found that nothing failed if the isolation regressed, since
   CI and a clean laptop give it nothing to undo. Three options were put to the
   Architect: all four routes, with a temp config file; the two environment routes
   only, with no file; or no regression test, recorded as a follow-up. All four were
   chosen. It checks only this file's own setup: it judges no other test and binds
   no future author, so no ledger row was filed for it. The per-route premise block
   is part of the same choice. Without it, a route git ignores would leave a mutant
   on that route's isolation line able to survive.
8. **The match is case-sensitive (Architect's choice, 2026-10-11).** Review proposed
   ignoring case, because Git for Windows reads `git_config_count` as
   `GIT_CONFIG_COUNT`. That is true of git, but no case can see it. On Windows,
   vitest 4.1.11 copies every variable to its uppercase name when it builds a
   worker's environment (`node_modules/vitest/dist/chunks/cli-api.*.js`: "env are
   case-insensitive on Windows, but spawned processes don't support it"). In a
   worker, a host's `git_config_count` arrived as `GIT_CONFIG_COUNT`, and npm's
   `npm_config_*` as `NPM_CONFIG_*`. git on POSIX ignores lowercase names. So
   ignoring case was an equivalent mutant: with a lowercase rewrite injected the
   file passed 12/12 with the flag and without it. It was removed. The premise is
   vitest's: if a later vitest stops uppercasing, a lowercase host variable would
   reach git on Windows, and nothing here would notice.
9. **Stubs per case, not per block (review).** `beforeEach`/`afterEach` instead of
   `beforeAll`/`afterAll`, so that turning on vitest's `unstubEnvs` (§3) cannot
   silently undo them.

## 6. Verification

Run on Windows 11 (10.0.26300), git 2.53.0.windows.2, Node v20.16.0, vitest 4.1.11,
from a session worktree whose `node_modules` was a junction to the main checkout's
(same lockfile, 328 packages, 0 version mismatches).

**Each config source, before and after.** Five ways to inject the same rewrite,
`url.https://github.com/.insteadOf = git@github.com:`. "Before" is the file as it is
on `main`. "After" is the first fix, `2a9eba5`, which added the isolation without the
hostile host. The final code is covered under "The final code" below.

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

**The final code** (`2e6e17d`, with the hostile host). `repo-remotes.test.ts` gives
12 passed in each of three conditions: nothing injected, the observed
`GIT_CONFIG_COUNT` rewrite, and the same rewrite spelled in lowercase
(`git_config_count`). With `--reporter=verbose`, the four premise cases read "rewrites
the push URL through 'GIT_CONFIG_COUNT' alone", "… 'GIT_CONFIG_PARAMETERS' alone",
"… 'a global file' alone" and "… 'a system file' alone", all passing.

**The mutation round** (`node scripts/mutate.cjs test/mutation/repo-remotes-isolation.json`,
TEST-STRATEGY §10). It ran in a detached worktree outside OneDrive, at the commit
under test, with `node_modules` junctioned. Final run, at `2e6e17d`:

```text
anchors: 7 of 7 match exactly once
baseline: PASS — 1 file, 12 passed: test/main/repo-remotes.test.ts
  control-comment  control  SURVIVED — certifies the round  [1 file, 12 passed]
  no-isolation  killed — 1 tests failed  [1 file]
  parameters-kept  killed — 1 tests failed  [1 file]
  count-kept  killed — 1 tests failed  [1 file]
  system-file-read  killed — 1 tests failed  [1 file]
  global-file-read  killed — 1 tests failed  [1 file]
  system-route-inert  killed — 1 tests failed  [1 file]
ROUND OK — 6 of 6 real mutants killed; the control survived.
```

The same spec gave the same result at `acabf73`, before the case flag was removed.
`git status` was clean after both runs. `system-route-inert` is killed only by the
premise block, which shows that block can fail.

**The case flag, measured** (§5.8). Each run had the rewrite injected in lowercase
(`git_config_count`, `git_config_key_0`, `git_config_value_0`):

| File | Result |
|---|---|
| `main`'s, with no isolation (the control) | 1 failed, 7 passed: the lowercase names reach git through vitest |
| `acabf73`, with the `i` flag removed by hand | 12 passed |
| `acabf73`, as committed (with `i`) | 12 passed |

A throwaway test (deleted afterwards) printed the worker's variable names. The host's
`git_config_count` arrived as `GIT_CONFIG_COUNT`. After the case-sensitive isolation,
`git config --get-regexp '^url'` found nothing. Neither Node (`spawnSync`, `fork`) nor
the `npx` shim changes the case. vitest's pool does
(`for (const name in env) env[name.toUpperCase()] = env[name]`, Windows only).

**Gates on the first fix, before the merge:**

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

**Full suite.** The runs were at default parallelism with 0.7–2.8 GB free. All but the
last were `npx vitest run` in this OneDrive worktree.

| Tree | Result |
|---|---|
| this change, on `75ec37c` | exit 1: 1 failed, 4918 passed, 8 skipped, 239 files |
| the same, rerun | exit 1: the same |
| `main`'s `repo-remotes.test.ts` restored (`src`, `shims`, `scripts`, `test` identical to `75ec37c`) | exit 1: the same |
| this change merged with `origin/main` `c0e1534` (tree `d0e750d`) | **exit 0: 240 files, 5006 passed, 8 skipped** |
| the final code, `2e6e17d` (also on `c0e1534`) | exit 1: 1 failed (the same timeout, 20 058 ms), 5009 passed, 8 skipped, 240 files |
| the head, `daf8b94` (merged with `origin/main` `8a3a3a7`), as `npm run test:coverage` in a detached worktree **outside** OneDrive | exit 1: 1 failed (the same timeout, 20 208 ms), 5021 passed, 8 skipped, 241 files |

In the five red runs, the one failure is `check-invariants.test.ts` › "the CLI, run
as CI runs it" › "exits 0 over this repository". Its `spawnSync` of
`scripts/check-invariants.cjs` hits `CHILD_TIMEOUT_MS` (20 s) with `ETIMEDOUT`, at
20 058–20 208 ms. Run alone, the checker exits 0 in 3 226 ms, and that test file
passes 20/20. The control run failed the same way without this change, so this change
did not cause it. On the first merged tree (`d0e750d`) it passed. `origin/main`'s only change to the
checker since `75ec37c` is one allowlist entry, so the timeout depends on load rather
than happening every time. The last run rules out OneDrive as the cause: outside it,
under coverage, the test timed out the same way. Because a test failed, vitest wrote no
coverage report, so `check-coverage` could not measure the floors here (it reported
1.04 GB free). The floors and a green full suite are left to CI's linux run. The
timeout is recorded here and not investigated further.

On the head `daf8b94` (tree `8322a2e`), in the same worktree outside OneDrive:
`npm run typecheck`, `npm run lint`, `check-invariants` (189/199),
`check-readme-current`, `check-attribution` (612 commits), CI's relative-link check
and `mutate.cjs … --check` (7 of 7 anchors) all exit 0.

On the first merged tree (`d0e750d`, before the hostile host), `npm run typecheck`,
`npm run lint`, `check-invariants`, `check-readme-current`, `check-attribution` (603
commits) and CI's relative-link check all exit 0. `repo-remotes.test.ts` gives 8
passed both with and without the injected `GIT_CONFIG_COUNT` rewrite.

**To reproduce:**

```bash
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0='url.https://github.com/.insteadOf' GIT_CONFIG_VALUE_0='git@github.com:' npx vitest run test/main/repo-remotes.test.ts
```

**Not proven:**
- **No vitest run on Linux was observed here.** Only plain Linux git was run, above.
  CI's ubuntu run now plays the hostile host on every push, so it exercises the
  isolation without anyone injecting anything; this note records no such run.
- **Only git 2.53 was run.** Git older than 2.32 ignores `GIT_CONFIG_GLOBAL` and
  `GIT_CONFIG_SYSTEM`. There, the premise cases for the file routes would fail
  loudly, and a `~/.gitconfig` rewrite would reach the other cases.
- **The case-sensitive match rests on vitest 4.1.11 uppercasing worker environments
  on Windows** (§5.8).

## 7. Related docs

- `test/main/repo-remotes.test.ts`: the comment above the block.
- `src/main/git.ts`: `ExecGitRunner`, `readRemotes`, `IDENTITY_ARGS`.
- `test/mutation/repo-remotes-isolation.json`: the round's spec.
- `docs/DECISIONS-LOG.md`: 2026-08-26 (the Agora's isolation from the machine's
  config), and this change's two entries.
- `docs/implementations/2026-10-02-pacing-wakes-teardown.md` §6: the rejected
  "config only in tests" option, distinguished in §5.4 above.
- `docs/TEST-STRATEGY.md` §2 (real git in temp directories) and §10 (the mutation
  round).

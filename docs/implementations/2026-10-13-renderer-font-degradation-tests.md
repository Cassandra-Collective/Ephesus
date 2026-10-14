# A missing pixel face is proved visible, and PR #74 lands as the contributor's

**Date:** 2026-10-13 · **Branch:** the contributor's
`erenbati:feature/renderer-font-degradation-tests-11`, by maintainer edit, from its
head `4868c2c` (which contains `main` at `8a3a3a7`)

---

Issue #11 asked for tests of `src/renderer/src/fonts.ts`, the module that loads the
three bundled pixel faces and reports the ones that did not load. A contributor
delivered them in PR #74. The tests mount the real `App` and read the status strip,
not the loader's return value. They cover all four ways a face can be lost.

Review (2026-10-12) found the tests right, with one problem in the branch's history and seven
around the tests. This change finishes the PR on its own branch: one Architect
commit goes on top of the contributor's seven, so their PR is the one that merges.
The commit closes the review's minor and nit findings. #74 is squash-merged because
four of its commits fail `prettier --check`.

## 1. Problem / motivation

**Why the tests matter.** UI-DESIGN §3 requires three faces, bundled, and
`scripts/sync-fonts.cjs` deliberately lets an install finish when one is missing.
Its header says "a missing package is NOT an error here: fonts.ts already reports
missing faces". That makes `fonts.ts` a degradation path, and BUILD-PROMPT §3.7
requires every degradation to be visible. Before #74, nothing proved the report ever
reached the screen. `src/renderer/src/fonts.ts` sat in the `untested` list of
`scripts/coverage-floors.json` on both platforms.

**What review found around the contributor's tests**, at the PR head `4868c2c`:

| Severity | Where | Finding |
|---|---|---|
| major | commits `94ecbfb`, `e980417`, `2842eab`, `1c24b05` | Each fails `prettier --check` on the test file, so `npm run lint` is red at each. CI built only the merge commit, and `main` lands PRs with merge commits. |
| minor | the non-WOFF2 fixture | `<htm` shares no byte with `wOF2` at any position. A signature check weakened to `some`, or to the first byte, stays green (§4). |
| minor | PR description | It does not name the production call path by file and line (CONTRIBUTING §4 rule 5, ENGINEERING-STANDARDS §6.7). |
| minor | the file's top | There is no header. Nothing ties the file to UI-DESIGN §3, BUILD-PROMPT §3.7 or #11, and nothing says why each child is mocked. The `ConsentGate` mock is one its sibling `bridge-heartbeat.test.tsx` does without. |
| nit | `afterEach` | It deletes a `window.eph` the file never sets (copied from `bridge-heartbeat.test.tsx`). |
| nit | the one-missing case | It is the only failure case without `not.toContain('fonts: bundled')`. |
| nit | `strip()` | It read the whole window's text, not the strip. |
| nit | merge commit `4868c2c` | Its subject is git's default (92 characters), not a Conventional Commit. |

The major is about history, not code. The last nit disappears with the squash.

## 2. What changed

| File | Change |
|---|---|
| `test/renderer/fonts.test.tsx` | Adds a header comment and a `wOFF` (WOFF1) case. The fixture bytes become named constants, and the helper takes a `signature` instead of an `invalidSignature` flag. `strip()` and `missingNames()` now read the `<header>`. Also: the `ConsentGate` mock is dropped, the one-missing case gains its negative assertion, and the stray `window.eph` cleanup is removed. |
| `docs/DECISIONS-LOG.md` | One entry appended: how #74 is finished and merged, with the Architect's four answers. |
| `docs/implementations/2026-10-13-renderer-font-degradation-tests.md` | This document. |

No production file changes. `scripts/coverage-floors.json` is not touched (§5).

## 3. Implementation approach

### The test file

The contributor's structure stays as it was:
- the `FakeFontFace` and `fetch` stubs;
- `mount()` under `act`;
- the five cases and their assertions.

The changes are:

- **A near-miss signature case.** The fixtures are now named constants (`WOFF2`,
  `HTML` and `WOFF1`), and `stubFontLoading` takes the bytes every request answers
  with. The new case answers every request with `wOFF`. `FakeFontFace.load()` accepts
  any bytes, so only the signature check in `fonts.ts` can turn those faces away.
  The case asserts all three faces are reported missing: in the strip text, in the
  `missing:` title, and by the absence of the healthy state.
- **The strip, not the window.** `header()` returns the `<header>` App renders and
  throws when there is none. It does not fall back to an empty string, because an
  empty string would make every `not.toContain` pass. `strip()` and `missingNames()`
  read through it.
- **One mock fewer.** `ConsentGate` (`src/renderer/src/ConsentGate.tsx`) returns from
  its effect when `window.eph?.consent` is absent, and renders `null` while its view
  is `null`. Without a bridge, as in every case here, it renders nothing, so the
  real component stays. The remaining five mocks are explained in the header:
  - `FloorCanvas` is Pixi, and it renders on the default tab.
  - `AgentPanel` imports xterm through `TerminalPanel`.
  - `AgentDock` and `CommandBar` render only on a ready bridge.
  - `AutonomyBadge` sits inside the strip and would add text of its own.
- **The header** names the production call path the tests run along:
  1. `src/renderer/src/main.tsx:11` renders `App`;
  2. `src/renderer/src/App.tsx:142` calls `loadPixelFonts`;
  3. `App.tsx:389–399` draws the result, as `⚠ fonts: n of 3 pixel faces missing`
     with a `missing:` title, or `● fonts: bundled`.

### How the PR is finished

#74's branch lives on the contributor's fork. The PR allows maintainers to push, so
the Architect's commits go on top of its head `4868c2c`, fast-forward only. Nothing
the contributor pushed is rewritten:

| Commit | What |
|---|---|
| `test(renderer):` | the test-file changes above |
| `docs:` | DECISIONS-LOG and this document |

The four Prettier-red commits cannot be removed without rewriting a branch that is
not ours, so **#74 is squash-merged**, as #60 was (DECISIONS-LOG 2026-10-02). Only
one commit reaches `main`, and its tree passes lint.

The squash commit is authored by the PR's author, with the Architect as co-author.
That is GitHub's default for a squash, and the shape #60 landed in (`cab9e1b`).
`check-attribution.cjs` accepts it.

The call path goes in the test header, this document and the squash commit body,
plus a maintainer comment on #74. The contributor's description is not edited.

Pushing dismisses the Architect's earlier approval: the `main` ruleset has
`dismiss_stale_reviews_on_push: true`. The Architect re-approves after reading the
commit. The ruleset does not require approval of the last push.

## 4. Mathematical / statistical details

### Why `<htm` could not catch a weakened check

`fonts.ts` accepts a file when its first four bytes equal WOFF2's, position by
position. Write `h` for the file's first four bytes and `m` for the signature:

```text
accept(h) = ∀ i ∈ {0,1,2,3} : h[i] = m[i]          m = 77 4f 46 32  ("wOF2")
```

Two plausible weakenings:

```text
some(h)  = ∃ i ∈ {0,1,2,3} : h[i] = m[i]            ("every" mistyped as "some")
first(h) = h[0] = m[0]                                ("check the leading byte")
```

A test fixture can only catch a weakening if it is rejected by the real check and
accepted by the weak one.

The fixture `<htm` is `3c 68 74 6d`. Position by position, it differs from
`77 4f 46 32` everywhere, so it agrees nowhere:
- `accept`, `some` and `first` are all false;
- all three checks reject it;
- no assertion on it can tell them apart.

`wOFF` is `77 4f 46 46`. It agrees at positions 0, 1 and 2 and differs only at 3:
- `accept` is false, so the real check rejects it;
- `some` and `first` are both true, so either weakening would load the face.

So the new case fails under both weakenings and passes under the real check. A
weakening that compared only the first three bytes would also accept `wOFF`.

### Coverage figures from CI on the contributor's head

These are from CI, before this finish. Run 38148637575 is on `refs/pull/74/merge`.
The base is `main` push run 38139128199 at `8a3a3a7`. Both ran on Linux with node
v20.20.2.

| Subsystem | Metric | `main` | PR head | Δ |
|---|---|---:|---:|---:|
| panels | lines | 46.79 | 48.21 | +1.42 |
| panels | branches | 46.69 | 47.09 | +0.40 |
| panels | functions | 34.56 | 35.29 | +0.73 |
| panels | statements | 44.91 | 46.40 | +1.49 |
| boot (`App.tsx` is a member) | lines | 27.94 | 28.06 | +0.12 |

Test totals went from 241 files, 5011 passed / 15 skipped, to 242 files, 5016 passed
/ 15 skipped. The skipped count did not move. `check-coverage.cjs` reported
`now tested: src/renderer/src/fonts.ts`.

The finish adds one case and changes no production file. Every line `fonts.ts` has
was already reached, so these figures should not move; CI on the finished head
confirms or refutes that.

## 5. Design decisions

- **Finish on the contributor's branch, not a superseding PR.** This follows the
  2026-10-02 precedent: the fix is the contributor's, so their PR is the one that
  merges.
- **Squash, not merge commit.**
  - A merge commit would put four lint-red trees on `main`.
  - Asking the contributor to squash would have meant a force-push on their branch
    and another review round, for a result the squash gives directly.
- **Author kept as author.** Rejected: the Architect as author, with the contributor
  as a `Co-authored-by` trailer. The work is theirs, and #60 set the shape.
- **The call path is recorded beside the code, not in the contributor's
  description.** It goes in the test header, this document, the squash body and a
  maintainer comment.
- **`ConsentGate` unmocked rather than explained.** An explanation would have had to
  say the mock does nothing. Removing a mock that changes nothing puts one more real
  component under the test.
- **`header()` throws instead of returning `''`.** With an empty string, a missing
  strip would satisfy every negative assertion. That is a silent fallback, and
  BUILD-PROMPT §3.7 forbids it in the tests as much as in the app.
- **The coverage record is not updated here.**
  - `--update` removes `fonts.ts` from `untested` and ratchets `panels` and `boot`.
  - It needs three corroborating runs per platform, and only upstream branches get
    push runs.
  - The fork branch therefore cannot provide them, and the ratchet stays a separate
    change.
  - The checker passes meanwhile: the drift is 1.42 points, inside the 5-point
    `ratchetLag`.

## 6. Verification

### Static checks on the finish commit

Windows 11 (10.0.26300), node v20.16.0; `.nvmrc` pins 20.20.2. `node_modules` is
linked from the main checkout; its 328 locked packages match `package-lock.json`.

```bash
npx prettier --check test/renderer/fonts.test.tsx
npx eslint --max-warnings 0 test/renderer/fonts.test.tsx
npx tsc --noEmit -p tsconfig.web-test.json
node scripts/check-invariants.cjs
```

Results: Prettier "All matched files use Prettier code style!"; ESLint clean; tsc
exit 0; `invariants ok (src, shims, scripts, test; reachability 189/199 src modules
reached, 10 unreachable by recorded decision, 6 type-only)`.

### Differential mutation round

**Condition.** Run by hand against `test/renderer/fonts.test.tsx` on the finish
commit `31ff93a`, on the same Windows condition. "Old" is the test file at the PR
head `4868c2c`; "new" is the finished file.

`scripts/mutate.cjs` needs its spec committed to the repository, and a review must
not commit. So the round kept the tool's safeguards by hand:
- the baselines must be green: old 5/5, new 6/6;
- a no-op control must survive on both trees;
- every run is scored from vitest's JSON report, and a run with zero tests is
  INVALID;
- `git checkout --` restores each file;
- each round file is hashed against HEAD after every run.

The suite's own 1 GB free-memory gate held the round back until memory was freed.
All round files were byte-identical to HEAD at the end.

| Mutant (in `src/renderer/src/fonts.ts` or `App.tsx`) | Old tests | New tests |
|---|---|---|
| no-op control (a comment word) | survived | survived |
| `fonts.ts:48` `every` → `some` | **survived** | **killed** by the WOFF1 case |
| `fonts.ts:48` first byte only, `(head[0] === WOFF2_MAGIC[0])` | **survived** | **killed** by the WOFF1 case |
| `fonts.ts:45` drop the `!response.ok` throw | — | killed (one-missing case) |
| `fonts.ts:51` `font.load()` not awaited | — | killed (`load()`-rejects case) |
| `fonts.ts:54` the catch rethrows | — | killed (5 of 6 cases) |
| `fonts.ts:53` drop `loaded.push` | — | survived |
| `App.tsx:392` `join(', ')` → `join(',')` | — | killed (both all-missing cases) |
| `App.tsx:394` count `.loaded` instead of `.missing` | — | killed (5 of 6 cases) |

**The first two rows are the point of this change.** Each weakened check passes the
old file and fails the new one. The difference is exactly the WOFF1 case.

**The first-byte mutant was run twice.** The first attempt replaced the call without
parentheses. Under the line's leading `!`, that read as `!head[0] === …`, which is
always false, so it accepted every file. It was killed by the HTML case for that
reason, so that run measured nothing. It is not counted. The table shows the
parenthesised re-run, which had its own green baselines and surviving control.

**The one survivor, `loaded.push`, is equivalent through the shipped entry point.**
`App.tsx` reads only `fonts.missing`, so no test that reads the strip can see
`loaded`. That is a question about `FontStatus`, not about these tests, and it is
left to the Architect.

### On CI, after the push

```bash
gh pr checks 74
```

The three required checks must be green on the finished head before the squash.

## 7. Related docs

- [UI-DESIGN](../design/UI-DESIGN.md) — §3, the three bundled faces
- [BUILD-PROMPT](../../BUILD-PROMPT.md) — §3.7, every degradation visible
- [ENGINEERING-STANDARDS](../ENGINEERING-STANDARDS.md) — §1 (lint and typecheck), §6.7 (the seam rule and the call path)
- [CONTRIBUTING](../../CONTRIBUTING.md) — §4 rule 5, the call path in the evidence
- [TEST-STRATEGY](../TEST-STRATEGY.md) — §2 (the per-subsystem ratchet), §10 (mutation rounds)
- [eph-usage and PR #60](2026-10-01-eph-usage-in-process-coverage.md) — the precedent for finishing a contributor's PR on its own branch
- [DECISIONS-LOG](../DECISIONS-LOG.md) — the 2026-10-02 entry on #60, and the 2026-10-13 entry on #74
- Issue [#11](https://github.com/Cassandra-Collective/Ephesus/issues/11) and PR [#74](https://github.com/Cassandra-Collective/Ephesus/pull/74)

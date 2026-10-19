# The Rust rewrite is decided, planned and measured before the first line of Rust

**Date:** 2026-10-19 · **Branch:** `feature/rust-rewrite` (on `main` @ `9269eaf`) ·
**Decisions:** DECISIONS-LOG 2026-10-19 (four entries) · **ADR:** ADR-0041 ·
**Gymnasium:** GYM-013 (proposed)

## 1. Problem / motivation

The Architect directed a rewrite of Ephesus in Rust on 2026-10-19. The TypeScript build
had become slow to extend and heavy to run, and he wants a fast harness that can
compete in the open-source field. A rewrite of about 59.3k lines of production
TypeScript (22.3k in `src/main`, 19.5k in `src/shared`, 9.1k in the renderer, plus
engines, watch, herald, harbor and preload), with 242 test files and about 5,000
tests, collides with the record in five places. Each needed the Architect, not the
engineer:

1. ADR-0001 is accepted and chose Electron. It has to be superseded, never edited.
2. Invariant 10, a decision memo per dependency, cannot hold unchanged when every
   capability needs a crate.
3. The Phase 9 roadmap is approved for planning on the TypeScript tree.
4. `main`'s three required checks are TypeScript gates under a ruleset only the
   Architect can edit.
5. Any change to what a gate refuses needs a Gymnasium row before the code
   (the 2026-09-02 ruling).

"Faster" was also a claim with no number behind it. Once the TypeScript build is
deleted, its numbers can never be measured again.

## 2. What changed

| File | Change |
|---|---|
| `docs/adr/ADR-0041-ephesus-is-rewritten-in-rust.md` | New. Supersedes ADR-0001. Records the eight decisions, the process architecture, the invariant translation table and the ADRs whose mechanism changes. |
| `docs/adr/README.md` | ADR-0041 indexed. ADR-0001's status reads "superseded by ADR-0041" (the index is not an ADR). |
| `docs/RUST-PLAN.md` | New. The rewrite's order R0 → R14, each phase's exit criteria, the crate map and dependency rules, the gate chain, the frozen-tree rules, risks, decisions owed and the proposed SRS amendments. PROPOSED. |
| `docs/RUST-CRATES.md` | New. The decision memo for the vetted crate set (Set A for approval, Set B listed and not approved) and the license allowlist. PROPOSED. |
| `docs/gymnasium/proposals/GYM-013-the-rust-rewrite-is-measured.md` | New. The umbrella row: the baseline and four falsifiable metrics. Proposed. |
| `docs/gymnasium/LEDGER.md` | GYM-013's row. |
| `docs/DECISIONS-LOG.md` | Four 2026-10-19 entries: the direction, both rounds of answers, and the engineer's conventions (branch, ADR and GYM numbers). |
| `docs/PROGRESS.md` | The *Resume here* block names the rewrite as the next step. A milestone row and a section with R0's three boxes and R1–R14's phase boxes. |
| `CLAUDE.md` | Says the system is being rewritten in Rust, points the resume at RUST-PLAN, and adds `cargo xtask gate`. |
| `docs/implementations/2026-10-19-rust-rewrite-direction.md` | This document. |

No code changed. Nothing under `src/`, `test/`, `shims/` or `scripts/` was touched.

## 3. Implementation approach

**Questions before writing.** Two rounds of four questions through AskUserQuestion,
batched so the Architect was interrupted twice, not eight times. The first round set
the crate layout (shell, strategy, location, dependency policy). The second could only
be asked after it: the GUI framework, what remains the contract, the TypeScript tree's
fate, and the Gymnasium. One thing his message had already settled was not asked: every
existing part comes first, and the roadmap resumes after.

**The GUI options were checked, not remembered.** Before the GUI question, the
crates.io API gave each candidate's version, license, release date and downloads, and
whether a maintained terminal widget exists. That check is why Slint (GPL-3.0 or
proprietary terms) and gpui (pre-1.0) were not offered. It is also why the context
said egui's own terminal widget is a 0.1 with about 1k downloads, so the widget is ours
to build.

**Every Set A crate was checked the same way** (§4), so the memo carries
read-on-the-day figures, not recollection.

**The baseline was measured before anything was frozen.** The TypeScript build was
compiled in this worktree (`npx electron-vite build`, 13.2 s) with the main checkout's
`node_modules` junctioned in. All 353 locked packages were first compared against
`package-lock.json`, with 0 mismatches. `src/` at `9269eaf` is identical to `8a3a3a7`,
which the donor tree was installed at. The method is §6.

## 4. Mathematical / statistical details

**Baseline aggregates.** With three runs $r_1, r_2, r_3$, $r_1$ is the cold run (first
boot after a build: file cache cold, antivirus scanning new files) and is reported but
excluded. The baseline is the warm mean $\bar{x} = (x_2 + x_3)/2$:

- start → control surface: $(1.499 + 1.551)/2 = 1.525 \approx 1.53$ s
- private bytes: $(269.6 + 269.4)/2 = 269.5$ MB

Two warm runs give no variance worth quoting. GYM-013 therefore states thresholds as
ratios that need no variance (≤ 50 % for start and memory, ≤ 10 % for footprint), and a
difference that large cannot come from run-to-run noise of a few percent. Run-to-run
spread was 3.4 % for start ($|1.551 - 1.499| / 1.525$) and 0.07 % for private bytes.

**Footprint** is the sum of three measured directories: Electron's `dist` (318 MB), the
built `out/` (4 MB), and the runtime dependency closure. The closure is every path
`npm ls --omit=dev --all --parseable` lists, realpath-deduplicated (328 packages),
summed file by file without descending into nested `node_modules`, which are listed
separately: 382.9 MB. Total 705 MB. A packaged installer would prune some of that,
but no installer exists, so the figure is what it takes to run the app today.

**Private bytes vs working set.** Private bytes (`PrivateMemorySize64`) is memory
committed to the process alone. Working set includes shared pages (DLLs, mapped
files), so it double-counts across the four Electron processes. GYM-013 uses private
bytes. Working set is reported only.

**Selection of the measured process tree.** The tree is the started PID plus every
descendant found by walking `Win32_Process.ParentProcessId` to a fixed point. Other
Electron apps on the machine (this desktop app is one) are not counted.

## 5. Design decisions

- **ADR-0041 supersedes ADR-0001 only.** ADR-0002, ADR-0007, ADR-0014 and ADR-0036 keep
  their decisions. Only the mechanism they name changes, so superseding them would have
  recorded a reversal that did not happen. Their mechanism changes are a table in
  ADR-0041.
- **The invariant table lives in the ADR, not in ENGINEERING-STANDARDS yet.** The
  standards doc gets its Rust edition in R0.3, once the gates exist to cite. Until then
  the ADR is the decision and the plan is the build order.
- **Enforcement moves from regex to structure.** Memory records three tripwires that a
  Prettier wrap or a name change got past. The Rust rules use the type system
  (`Secret` cannot be printed), the Cargo graph (`eph-ui` cannot reach I/O) and `syn`
  for what neither can express.
- **R4, a first window, comes before Hermes.** Terminal fidelity is the highest risk,
  and the Architect should see a Rust Ephesus early. The plan is risk-first inside the
  dependency order.
- **Thresholds are proposed, not decided.** GYM-013's 50 %, 50 % and 10 % are the
  engineer's proposal for the Architect to amend. A metric he did not choose is not his
  definition of "faster".
- **No time estimate anywhere.** Memory: *quote a cost only after you have summed it*.
  The plan counts phases and packages, not weeks.
- **The branch moved before the first commit.** The worktree's original branch carried
  a vendor word. CLAUDE.md fixes the `feature/` prefix, so this was convention, not a
  question.

## 6. Verification

**Baseline (tier A, this session):** win32 (Windows 11 Home 10.0.26300), Electron 37
(`node_modules/electron`), built app at `9269eaf`, fresh scratch home per run under
`%TEMP%`, a scratch Chromium profile (`--user-data-dir`), consent not granted, 30 s
after start:

```
run 1 : ready_ms=11057 procs=4 workingset_mb=434.6 private_mb=281
run 2 : ready_ms=1499 procs=4 workingset_mb=520.8 private_mb=269.6
run 3 : ready_ms=1551 procs=4 workingset_mb=523.5 private_mb=269.4
```

"Ready" is the moment `<home>/control-endpoint.json` exists. The harness writes it
once the control socket is serving (SDD §2), and the Rust build will serve the same
surface, so the same event can be timed in both. Not proven: that the window had
painted by then (the TypeScript build logs no paint event), and anything about
Linux or macOS. Each scratch home was deleted after its run, and `~/.ephesus` was
not touched.

The script, re-runnable as it was run:

```powershell
param([string]$App = '<worktree path>', [int]$Runs = 3, [int]$IdleSeconds = 30)
$electron = Join-Path $App 'node_modules\electron\dist\electron.exe'
function Get-Tree([int]$root) {
  $all = Get-CimInstance Win32_Process
  $ids = [System.Collections.Generic.List[int]]::new(); $ids.Add($root)
  $changed = $true
  while ($changed) {
    $changed = $false
    foreach ($p in $all) {
      if ($ids.Contains([int]$p.ParentProcessId) -and -not $ids.Contains([int]$p.ProcessId)) {
        $ids.Add([int]$p.ProcessId); $changed = $true }
    }
  }
  return $ids
}
for ($i = 1; $i -le $Runs; $i++) {
  $base = Join-Path $env:TEMP "eph-baseline-$i-$([guid]::NewGuid().ToString('N').Substring(0,6))"
  $home_ = Join-Path $base 'home'; $ud = Join-Path $base 'userdata'
  New-Item -ItemType Directory -Force $home_, $ud | Out-Null
  $env:EPH_HOME = $home_
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $proc = Start-Process -FilePath $electron -ArgumentList @("`"$App`"", "--user-data-dir=`"$ud`"") -PassThru
  $endpoint = Join-Path $home_ 'control-endpoint.json'; $ready = $null
  while ($sw.Elapsed.TotalSeconds -lt 60) {
    if (Test-Path $endpoint) { $ready = $sw.Elapsed.TotalMilliseconds; break }
    Start-Sleep -Milliseconds 20 }
  while ($sw.Elapsed.TotalSeconds -lt $IdleSeconds) { Start-Sleep -Milliseconds 250 }
  $ps = Get-Process -Id (Get-Tree $proc.Id) -ErrorAction SilentlyContinue
  $ws = ($ps | Measure-Object WorkingSet64 -Sum).Sum / 1MB
  $priv = ($ps | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB
  "run $i : ready_ms=$([math]::Round($ready)) procs=$($ps.Count) workingset_mb=$([math]::Round($ws,1)) private_mb=$([math]::Round($priv,1))"
  & taskkill /PID $proc.Id /T /F | Out-Null; Start-Sleep -Seconds 2
  Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
Remove-Item Env:EPH_HOME
```

**The record (tier C):** `git diff --stat` on the commit shows only the files of §2.
The TypeScript gates are unaffected: `docs/` and `*.md` are outside Prettier's scope
(`.prettierignore`), and nothing that ESLint, `tsc` or vitest reads changed.

**Not yet verified:** nothing in the plan is built. The plan, the crate set, the SRS
amendment wording and GYM-013's thresholds are proposals awaiting the Architect.

## 7. Related docs

ADR-0001 (superseded) · ADR-0041 · `docs/RUST-PLAN.md` · `docs/RUST-CRATES.md` ·
GYM-013 · `docs/PROGRESS.md` (*Resume here*) · BUILD-PROMPT §3 (the invariants) ·
`docs/srs/SRS.md` §4–§6 · `docs/TEST-STRATEGY.md` §3 and §10.

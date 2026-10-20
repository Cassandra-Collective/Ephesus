# The Rust rewrite — plan (APPROVED by the Architect 2026-10-19)

The decision is [ADR-0041](./adr/ADR-0041-ephesus-is-rewritten-in-rust.md). The
dependency memo is [`RUST-CRATES.md`](./RUST-CRATES.md), and the measurement is
[GYM-013](./gymnasium/proposals/GYM-013-the-rust-rewrite-is-measured.md). The decisions
behind all three are in [`DECISIONS-LOG.md`](./DECISIONS-LOG.md) under 2026-10-19.
This file is the build order. It replaces BUILD-PROMPT §5's M9 → M9h order **until
parity**, after which M9 resumes on the Rust tree (§7).

**The Architect approved this plan on 2026-10-19** (DECISIONS-LOG, same date): the order,
the exit criteria, R0's packages and §10's SRS wording. Each later phase's package list
is still put to him at that phase's start, through the questions workflow, as M-plans
were. The phases below fix the order and the exit criteria, not every package.

---

## 1. The Architect's direction, as claims the rewrite is held to

From his message of 2026-10-19 and the answers that followed:

1. Every component of the system is recreated in Rust. The window counts: an all-Rust
   GUI on egui/eframe.
2. Each part is verified working before the next is started. "Verify, continue,
   integrate and test more, then move to the next part."
3. A clean break for formats Ephesus owns. The SRS and the thirteen invariants remain
   the contract.
4. The TypeScript tree is frozen as the behavioural reference and deleted at parity.
5. Once everything built to date runs in Rust, the roadmap continues: M9, then
   Phase 9.
6. "Faster" is measured (GYM-013), not asserted.

## 2. What a clean break cannot break

These are the engine's facts, not ours. A Rust module that reads them is held to the
recorded ground truth (memory: *Claude Code engine ground truth*: real transcripts and
the binary settle what the engine does, never a guess):

- Claude Code's hook payloads (per event name), its `--settings` / `--setting-sources`
  flags, `--permission-mode`, `--resume`, `CLAUDE_CONFIG_DIR` and
  `CLAUDE_SECURESTORAGE_CONFIG_DIR`, `DISABLE_AUTOUPDATER`, the attribution settings,
  the per-workspace trust record, the transcript JSONL and the `prompt-submitted`
  acknowledgement.
- The permission-rule matcher's exact tool names (`Edit`, `Read`; see
  `docs/implementations/2026-09-05-mailbox-grant-rule-shape.md`).
- `git` (housekeeping detach, `--literal-pathspecs`, worktree semantics, hermetic config)
  and `gh` (JSON output, auth). GitHub's API rewrites `\u0000` in JSON responses.
- The fixtures that captured them stay where they are (`test/fixtures/engine-output/`,
  `test/fixtures/claude-limit/`) and are read by the Rust tests. They are evidence, not
  TypeScript.

## 3. Architecture

### 3.1 Crates

```
Cargo.toml                 workspace; [workspace.lints]; resolver 3
rust-toolchain.toml        pinned 1.96.0 (+ rustfmt, clippy)
clippy.toml                disallowed_methods / disallowed_types (ADR-0041 table)
deny.toml                  licenses + advisories (cargo-deny)
crates/
  eph-schema/    pure data models + validators, every persisted type `Versioned`
  eph-fs/        write_atomic, append-only JSONL, home layout + first-boot seeding
  eph-git/       THE single committer + worktrees (the only crate that runs git)
  eph-agora/     registry, ledger, board, the book of record, degradations, state stores
  eph-pty/       PTY manager on portable-pty; PATH resolution; output redaction
  eph-engines/   EngineAdapter trait; claude (reference); codex, gemini (conformance only)
  eph-hooks/     events.sock endpoint; per-spawn tokens; recall answering
  eph-hermes/    mail routing, hop cap, bounce, wake watchdog, Stop-hook decisioning
  eph-watch/     gates, budgets + cost ledger, breaker, secret broker, pacing, telemetry
  eph-library/   memory, FTS5 rung, MemPalace driver, reflection, knowledge shelf
  eph-odeon/     briefings, decks, memos, meetings, org metrics, retros
  eph-profiles/  mission profiles, scheduler + durable trigger clock, playbooks
  eph-harbor/    GitHub via gh, company identity, chat bridge, webhooks, hires
  eph-gym/       Gymnasium, Stoa, company modes + proof gate
  eph-herald/    voice seam, policy (repeat-back), providers
  eph-api/       the command/event surface (window AND control socket); pure types
  eph-core/      boot, wiring, restore, shutdown, control server — holds no domain logic
  eph-ui/        egui window, design tokens, panels, terminal widget, the floor
bins (in crates/eph-app/): ephesus (window), ephesusd (headless), ephctl, eph-hook,
  eph-recall, eph-usage, eph-gh-token
xtask/           the invariant checks (§4), run by `cargo xtask gate`
```

Crates are created when their phase starts, not up front. A crate that would hold
fewer than one package's worth of code is folded into its neighbour until it grows.

### 3.2 Dependency rules (checked by `xtask`, not by review)

- `eph-schema` and `eph-api` do no I/O and depend on no Ephesus crate except
  `eph-schema`.
- `eph-ui` depends on `eph-api`, egui/eframe and `alacritty_terminal` (whose `tty`
  module it must not use), and on nothing else of ours.
- Only `eph-git` constructs a `git` command. Only `eph-pty`, `eph-engines` (version
  probes), `eph-library` (MemPalace) and `eph-harbor` (`gh`) may construct any other
  process, each named in `clippy.toml`'s allowlist with its reason.
- Only `eph-fs` may write a file another process reads.

### 3.3 Process model

One core library. `ephesus` runs it with a window, and `ephesusd` runs it with none
(FR-15.6). Both serve `control.sock`. The window is an in-process client of the same
`eph-api` handlers, so the window and the CLI cannot drift (the ADR-0033 property
`ControlDeps = Pick<IpcDeps, …>` gave the TypeScript build).

## 4. The gate chain

`cargo xtask gate` runs, in order, and stops on the first red:

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings` (lints from `[workspace.lints]`
   and `clippy.toml`)
3. `cargo test --workspace`
4. `cargo deny check` (licenses, advisories, bans)
5. `xtask invariants`: the dependency rules of §3.2 from `cargo metadata`; the
   single-committer rule over the syntax tree (`syn`); the token rule for `eph-ui`; the
   prompt-literal rule.
6. `xtask deps`: every direct dependency is listed in `RUST-CRATES.md`.
7. From R1 on: coverage floors per crate (`cargo-llvm-cov`), ratcheted with the
   project's existing rules (three runs per platform, never a lowered floor).

CI runs the chain on `ubuntu-latest` and `windows-latest` in a new `rust` job. The
TypeScript jobs keep running over the frozen tree, unchanged, because they are `main`'s
required checks. Every check above that *refuses* something gets the refutation pass
before its package closes: a planted violation must fail it by file and line (memory:
*Refute a new gate before closing*).

`check-attribution.cjs` reads git history and is language-neutral. It stays.
`check-readme-current.cjs` parses only `M`-prefixed package ids, so a ticked `R`
package would pass it unseen. It learns the `R` prefix in R0.3. Both are gate changes
under GYM-013.

## 5. Phases

Order is risk-first inside the dependency order: the book of record before anything
that writes to it, the terminal before the window, and a first window (R4) before most
of the domain, because terminal fidelity is the rewrite's highest risk and the
Architect should see a Rust Ephesus early.

Every phase closes the way M-milestones closed: its exit criteria run, real output
shown, a mutation round with a control for every guard it adds, PROGRESS ticked in the
same commit, and an implementation doc.

### R0 — The workspace and its gates

- **R0.1** Workspace skeleton: `Cargo.toml`, `rust-toolchain.toml`, `[workspace.lints]`,
  `clippy.toml`, `deny.toml`, `.gitignore` for `target/`, and the documented
  `CARGO_TARGET_DIR` outside OneDrive.
- **R0.2** `xtask` with `gate`, `invariants` and `deps`, and the `rust` CI job on both
  platforms. CI starts the gate through the `--locked` alias (R0.1 §6). A mutation round
  becomes `cargo xtask mutants <crate>`, driven by a committed spec and running its
  control (a unit-tests-only pass that must report misses) inside the same command. That
  keeps GYM-008's landed metric (every round cites a checked-in spec) true for Rust.
- **R0.3** ENGINEERING-STANDARDS and BUILD-PROMPT §3 gain the Rust edition (ADR-0041's
  table). `check-readme-current.cjs` learns `R`. The skills (`/build-package`, `/goal`,
  `/milestone-review`) learn the R-order and the Rust gate.

*Exit:* the chain is green on win32 and on CI linux and windows, and each `xtask` rule
fails a planted violation by file and line.

### R1 — The home and the book of record

- **R1.1** `eph-schema`: the persisted models, redesigned where the clean break allows,
  each specified in the new SDD §4 first.
- **R1.2** `eph-fs`: `write_atomic`, the append-only JSONL writer (seq recovery, torn-tail
  tolerance), the home layout and first-boot seeding (`gate-policy.json`,
  `authority.json`: only when absent, reported).
- **R1.3** `eph-git`: the single committer (queue, retry and backoff, stale `index.lock`,
  startup reconcile, foreground housekeeping, literal pathspecs, hermetic config) and
  worktrees (FR-1.5's vacancy rule: nothing is deleted to make room).
- **R1.4** `eph-agora`: registry, ledger, board, the log with publish and subscribe, its
  readers, the degradation channel, `StateStore` (absent ≠ damaged), `DIAGNOSIS.md`.

*Exit:* FR-4.1, FR-4.2, FR-4.4 and NFR-7 by test. Killed mid-append and mid-commit,
nothing is lost or torn on reopen. A mutation round with a control.

### R2 — The terminal plane and the engine seam

- **R2.1** `eph-pty`: spawn, write, resize, interrupt, kill; PATH resolution; the
  redaction filter.
- **R2.2** `eph-engines`: the adapter trait, the `claude` adapter (spawn plan, per-agent
  config dir ADR-0026, workspace trust ADR-0021/0025, autonomy ADR-0031, attribution
  suppression FR-1.8, no self-upgrade ADR-0028, prompts declared in advance ADR-0035),
  and `codex`/`gemini` as unregistered conformance subjects (ADR-0024).
- **R2.3** The agent manager: spawn ordering, the install offer (FR-1.6), the exit unwind,
  the command queue (FR-1.3), the respawn ladder and crew survival.

*Exit:* a real `claude` runs in a PTY under a headless test binary with its own config
directory and suppressed attribution. The conformance table is green for every subject
in both directions. FR-1.1 to FR-1.10.

### R3 — The event plane

- **R3.1** `eph-hooks`: the endpoint (POSIX socket `0600`, Windows local named pipe),
  per-spawn tokens, payload validation, the schema-drift warning, recall answering.
- **R3.2** The shims as Rust binaries (`eph-hook`, `eph-recall`, `eph-usage`,
  `eph-gh-token`), so the product needs no Node at all.
- **R3.3** The avatar state machine (SDD §6) with its timers.

*Exit:* a real `claude` session's hook events drive the state machine. FR-2.1 to FR-2.3.
NFR-2's hook → state p95 ≤ 200 ms, measured and reported with its condition.

### R4 — The first window

- **R4.1** `eph-api`: the command/event surface as Rust types, served in-process to the
  window.
- **R4.2** `eph-ui` shell: the eframe window, design tokens (UI-DESIGN), the roster, the
  consent gate, the degradation strip.
- **R4.3** The terminal widget on `alacritty_terminal`: byte-for-byte rendering, input
  including Escape-as-interrupt, resize, scrollback, selection and copy.

*Exit:* the Architect types into a live `claude` agent in the Rust window. `egui_kittest`
drives the window in CI. NFR-1's keystroke → echo ≤ 50 ms over raw PTY, measured.
Screenshot evidence.

### R5 — Hermes

Mail as speech-act files, atomic delivery, the hop cap escalating to Artemis, bounce,
broadcast and `to:"human"`, the wake watchdog, Stop-hook decisioning (block cap,
`stop_hook_active`), cursor idempotency, stranded-mail disclosure.
*Exit:* FR-3.1 to FR-3.7, NFR-6, NFR-2 delivery p95 ≤ 500 ms, the livelock scenario.

### R6 — The Watch

Gates and policy (deny-all fallback), approvals, durable gates and verdicts, budgets
with the SQLite cost ledger and transcript reader, the breaker with durable rung-3
stops, the secret broker (write-only, env-injected, OS keychain seam), usage-aware
pacing, ceilings settable from the app, telemetry spans.
*Exit:* FR-11.1 to FR-11.7, FR-1.9, the breaker and gate scenarios.

### R7 — Artemis, the control surface, the headless company

The orchestrator's lifecycle and reserved seat, respawn with memory, delegated
authority, the ledger endpoint, `control.sock` and `ephctl`, the consent gate
(ADR-0032), one harness per home (ADR-0034), the restore order (ADR-0027), the quit
sequence.
*Exit:* a two-agent company directed through `ephctl` with no window (UC-02, FR-15.6).
SRS §6 criterion 6 (blackout) passes. GYM-013's headless checkpoint is measured.

### R8 — The Library

Memory files, the FTS5 rung, the MemPalace subprocess driver, the recall ladder,
reflection, the knowledge shelf. *Exit:* FR-6.1 to FR-6.4.

### R9 — The Odeon, the org and the profiles

Briefings compiled from Agora data, decks and the deck gate, memos and verdict routing,
meetings and minutes, org metrics and retros, the three mission profiles, the scheduler
with its durable trigger clock, playbooks, the activation preview.
*Exit:* FR-7.1 to FR-7.5 (speech excepted, R12), FR-9.1 to FR-9.5, FR-11.5. SRS §6
criteria 3 and 4.

### R10 — The Harbor

GitHub through `gh`, ingestion, the company identity as a GitHub App (ADR-0022), the
chat bridge, webhooks, hire export and import, remote tagging, outbound drafts that
survive with their gate (ADR-0030).
*Exit:* FR-10.1 to FR-10.5.

### R11 — The Gymnasium, the Stoa and company modes

*Exit:* FR-12.1 to FR-12.6, FR-13.1 to FR-13.7, FR-14.1 to FR-14.5. SRS §6 criteria 8
and 9 by their scenario suites. The Stoa seed decisions of 2026-10-11 (DECISIONS-LOG,
carried from closed PR #76) are requirements: the seed validates every row it parses
and reports each one it refuses, pattern intake needs a license on the MIT-compatible
list, and curation refuses to write over a watchlist it cannot read.

### R12 — The Herald

The seam, the policy (wake word, barge-in, FR-8.4's repeat-back), ElevenLabs and OpenAI
Realtime. In the TypeScript build the Herald is built but not wired into the app (M6.9,
deferred by the Architect). Parity is the same state unless he decides otherwise when
R12 starts. *Exit:* FR-8.1 to FR-8.6 at the level the TypeScript build reached.

### R13 — The floor and the full window

The Terraces floor (tiles, avatars, walking, envelopes, stations, seats; the licensed
art pack loaded when present, procedural tiles otherwise), every panel, keyboard
navigation, AccessKit, WCAG AA contrast.
*Exit:* NFR-1's 60 fps with 15 avatars, measured with its condition. NFR-14, NFR-15.
UC-03.

### R14 — Parity

Every FR the TypeScript build satisfied at M8c, re-demonstrated. SRS §6 criteria run on
the Rust build to the standard the TypeScript build met them, and no further. GYM-013's
metrics measured. The TypeScript tree deleted in one commit. The ruleset switched to the
Rust checks (the Architect's action). README, site and SDD say what is true.
*Exit:* the Architect's verdict. Then M9 resumes (§7).

## 6. The frozen TypeScript tree

- Nothing under `src/`, `test/`, `shims/`, `scripts/` (except the README-currency check's
  `R` prefix in R0.3) or the TypeScript configs changes until R14 deletes it.
- Its CI keeps `main`'s required checks satisfiable. A Rust PR passes them trivially
  because it touches nothing they read, and must also pass the `rust` job.
- When the Rust build reaches parity, the Architect switches the ruleset's required
  checks to the `rust` job before the deletion lands. If the deletion landed first, no
  PR could merge.
- It is the behavioural reference: when a phase's behaviour is in doubt, the TypeScript
  module and its tests are read before anything is designed.

## 7. After parity

M9 resumes on the Rust tree in its approved order (DD-M9-1, DD-M9-18): M9 → M9b → M9d →
M9e → M9f → M7b (v1), with M9c beside M9b and M9d, then M9g and M9h. Whether M9's build
may start is still the Architect's question at that point (PROGRESS's resume block).
`eph-agent` (ADR-0036) becomes a Rust binary in this workspace. Phase 9's plan is
re-read for TypeScript-specific mechanisms before its first package.

## 8. Risks

| Risk | Mitigation |
|---|---|
| Terminal fidelity below xterm.js (wide chars, mouse modes, bracketed paste, the engine's TUI) | Built at R4, before most of the domain. Measured against the real `claude` TUI, not a fixture. |
| ConPTY quirks on Windows (resize races, exit codes, CRLF) | `portable-pty` is WezTerm's layer. R2's exit runs a real engine on win32 and linux. |
| egui accessibility below the web renderer's | AccessKit is built into egui. NFR-15 is an R13 exit criterion, measured, not assumed. |
| The verification discipline lost in the move | `cargo-mutants` with a control, `cargo-llvm-cov` floors and `egui_kittest` are in the vetted set. Every gate is refuted before it closes. |
| The crate graph grows unseen (egui and wgpu pull many transitive crates) | `cargo deny` over the whole graph. `xtask deps` holds direct dependencies to the memo. |
| OneDrive and Cargo's `target/` | `CARGO_TARGET_DIR` outside OneDrive, documented in R0.1. |
| A long rewrite with no product shipped meanwhile | R4 puts a working Rust window in front of the Architect early. Each phase ends runnable. |

## 9. Decisions still owed (asked at their phase, not before)

- R0: the CI matrix (macOS included or not), and whether `cargo xtask gate` replaces the
  `npm` commands in CLAUDE.md now or at parity.
- R1: each redesigned schema (new SDD §4), field by field, where it departs from today's.
- R2: whether `portable-pty`'s process-tree kill suffices on Windows or a job object (an
  FFI boundary crate) is needed.
- R6: the OS keychain crate, a separate memo (`keyring` is the candidate).
- R10 and R12: the HTTP, WebSocket and audio crates, a separate memo each.
- R12: whether the Herald is wired into the app this time (M6.9).

## 10. SRS amendments (approved 2026-10-19; applied in R0.3's commit as dated amendments)

- **FR-1.1.** *Was:* "…in a dedicated PTY (node-pty), streaming output to an xterm.js
  view byte-for-byte." *Proposed:* "…in a dedicated PTY (ConPTY on Windows, openpty on
  POSIX — ADR-0041), streaming output to the window's terminal view byte-for-byte."
- **NFR-8.** *Was:* "Renderer runs sandboxed with contextIsolation; all fs/git access
  brokered through typed IPC." *Proposed:* "The window's code SHALL perform no file,
  git, process or network I/O itself. Every such action is a typed command handled by
  the core and validated there. The boundary is enforced by the crate dependency graph
  (ADR-0041)."
- **NFR-16.** *Was:* "Typecheck-clean TypeScript throughout…" *Proposed:* "Rust
  throughout, clean under the workspace's `fmt`, `clippy -D warnings` and `deny`
  configuration, with no `unsafe` outside named boundary crates. The standards doc's
  Definition of Done gates every merge."

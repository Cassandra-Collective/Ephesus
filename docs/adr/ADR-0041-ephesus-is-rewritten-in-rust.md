# ADR-0041 — Ephesus is rewritten in Rust, window included

**Status:** accepted · **Date:** 2026-10-19 · **Supersedes:** ADR-0001 ·
**Changes the mechanism, not the decision, of:** ADR-0002, ADR-0007, ADR-0014, ADR-0036

## Context

ADR-0001 chose Electron + React + TypeScript and rejected a Rust shell "for v1", with a
standing clause: *revisit if footprint becomes a real complaint (recorded as a potential
superseding ADR)*. This is that ADR.

On 2026-10-19 the Architect directed a rewrite of the whole system in Rust. His reasons,
in summary: the TypeScript build had become slow to extend and kept producing seams that
cost a package each to close, the shipped app had grown heavy, and Ephesus needs to be
fast to compete as an open-source agent harness. The directive is to recreate every
component in Rust, verify each one, integrate and test, then move to the next, and to
resume the product roadmap (Phase 9) once everything built to date runs in Rust.

What the TypeScript build costs, measured before it was frozen (win32, Electron 37, the
built app booted into a fresh scratch home, consent not granted so no agent spawned; three
runs, `docs/implementations/2026-10-19-rust-rewrite-direction.md` §6):

| Measure | Run 1 (cold) | Run 2 | Run 3 |
|---|---|---|---|
| Process start → control surface serving (`control-endpoint.json` written) | 11.06 s | 1.50 s | 1.55 s |
| Processes | 4 | 4 | 4 |
| Working set, all processes, 30 s after start | 434.6 MB | 520.8 MB | 523.5 MB |
| Private bytes, all processes, 30 s after start | 281.0 MB | 269.6 MB | 269.4 MB |

On disk, unpackaged (there is no installer): Electron 318 MB, the built app 4 MB, the
runtime dependency closure 383 MB (328 packages).

## Decision

The Architect decided each of the following through the questions workflow on
2026-10-19 (DECISIONS-LOG, same date).

1. **Every component is rewritten in Rust**: the core, the hook shims, the CLI, and the
   window. The TypeScript tree is frozen in place as the behavioural reference and
   deleted in one commit when the Rust build reaches parity (§Parity below). It is not
   developed further.
2. **The window is an all-Rust GUI on egui/eframe.** The terminal view is a widget
   Ephesus owns, built on `alacritty_terminal`'s emulator. The Terraces floor is drawn
   with egui's painter. Rejected: a Rust core under Tauri 2 that keeps the React/Pixi/
   xterm renderer; a Rust core with Electron as a thin client; iced (a maintained
   terminal widget, but a slower release cadence and a smaller ecosystem).
3. **A clean break for the formats Ephesus owns.** The Rust build does not have to read
   a TypeScript-era harness home. Its on-disk layout, schemas and wire protocols are
   redesigned where that serves the system, and each is specified in the new SDD before
   it is built. Rejected: a parallel build bound to the same home files; an in-process
   strangler loading Rust into Electron through napi-rs.
4. **What a clean break cannot break.** The engine is not ours. Claude Code's hook
   payload JSON, its CLI flags, its settings-file shape, its per-workspace trust record,
   its transcript JSONL and its config directory layout are external facts, and so are
   `git`'s and `gh`'s behaviour. The Rust build reads them as they are. The record of
   those facts is the engine ground truth already gathered (SDD §3, the adapter tests,
   `test/fixtures/engine-output/`, `test/fixtures/claude-limit/`).
5. **The contract is unchanged in kind.** The SRS (requirements, use cases, acceptance
   criteria) and the thirteen invariants remain the "what". The SDD is rewritten for
   the Rust architecture. This ADR supersedes ADR-0001 outright. ADR-0002, ADR-0007,
   ADR-0014 and ADR-0036 keep their decisions, and the mechanisms they name (two Electron
   tiers, provider SDKs, node-pty, a TypeScript build target for `eph-agent`) become
   their Rust equivalents, listed in Consequences. Every other accepted ADR is
   language-neutral and stands as written.
6. **One repository.** The Rust workspace is `crates/` in this repository, on a
   `feature/` branch. In the Architect's words: "Ephesus repository will be rewritten
   on top of the existing repo no need to open a new one the typescript version will
   not be continued". Cargo's build output lives outside OneDrive.
7. **Dependencies are approved as a vetted set.** One decision memo,
   `docs/RUST-CRATES.md`, lists the crates the rewrite starts with: license, version,
   role and the phase that needs each. The Architect approves it as a set. A crate
   outside the set gets its own memo before it is added. Invariant 10 is unchanged
   in force and is now checked by machine (Consequences, row 10). For this rewrite only,
   this supersedes DD-M9-9's rejection of approving a named set in one batch.
8. **The rewrite is measured.** The umbrella Gymnasium row GYM-013 carries the
   falsifiable meaning of "faster" against the baseline above.

## Process architecture (replaces ADR-0001's three tiers)

```
┌──────────────────────────────── ephesus (one process) ───────────────────────────────┐
│  eph-ui (egui window)  ──commands/events──►  eph-api  ◄──same handlers──  control.sock │
│       │ depends on eph-api only                 │                        (ephctl,     │
│       ▼                                         ▼                         scripts)     │
│  terminal widget (alacritty_terminal)     eph-core: boot, wiring, restore, shutdown    │
│                                           agora · hermes · watch · engines · pty ·     │
│                                           library · odeon · profiles · harbor · gym…   │
└───────────────────────────────────────────────────────────────────────────────────────┘
          ▲ PTY bytes                                ▲ hook envelopes (events.sock)
   ┌──────┴────────────┐                    ┌────────┴──────────────────────┐
   │ agent processes   │───────────────────►│ eph-hook (Rust binary shim)   │
   │ (claude, …) in    │                    └───────────────────────────────┘
   │ ConPTY / openpty  │
   └───────────────────┘
```

`ephesusd` is the same core with no window (FR-15.6). The window is a client of the
same handler set the control socket serves, so the two cannot drift. ADR-0033's rule is
unchanged: a script may run the company, and only a human may authorise.

## Options considered

- **Rust core + Tauri 2, keeping the web renderer.** Recommended, rejected by the
  Architect. Keeps React/Pixi/xterm, which have no Rust-native equivalents of the same
  maturity, but leaves the product in two languages.
- **Rust core + Electron as a thin client.** Least UI risk. Node stays in the product.
- **All-Rust GUI on iced.** A ready terminal widget (`iced_term`) and `iced_test`, but a
  slower release cadence (0.14, December 2025) and a smaller ecosystem than egui
  (0.37, October 2026, about 6.8 M recent downloads).
- **Slint, gpui.** Not offered. Slint is GPL-3.0 or proprietary-terms, and gpui is
  pre-1.0 and tied to Zed.
- **Parallel build on the same home files; in-process strangler (napi-rs).** Both keep
  the TypeScript app running throughout. The Architect chose a clean break.
- **A new repository.** Rejected: the documentation suite and the record continue here.

## Consequences

### The thirteen invariants in Rust

Each row keeps the invariant's force. Where the TypeScript build enforced a rule with a
regex tripwire, the Rust build enforces it through the type system, the Cargo
dependency graph or a syntax-tree check in `xtask`. The memories about per-line
tripwires missing wrapped calls are why.

| # | Invariant | How the Rust build holds it |
|---|---|---|
| 1 | Strict TS, no `any` past validators | `unsafe_code = "forbid"` workspace-wide. A crate that needs FFI is a named boundary crate with its own memo. `clippy::unwrap_used`, `expect_used`, `panic`, `todo` and `unimplemented` deny in non-test code. Untyped JSON (`serde_json::Value`) is confined to validator modules by `clippy::disallowed_types`. |
| 2 | The renderer never touches Node/fs | `eph-ui` may depend only on `eph-api`, egui/eframe and `alacritty_terminal`'s emulator, never on a crate that does I/O. `xtask` checks this from `cargo metadata`. The window renders agent text as data. egui runs no scripts, so THREAT-MODEL is rewritten for the new boundary rather than patched. |
| 3 | Atomic writes for shared files | `eph-fs::write_atomic` (temp file, flush, rename) is the one write path for anything another process reads. `std::fs::write`, `File::create` and truncating opens are `disallowed_methods` outside `eph-fs`. |
| 4 | Only main commits to the Agora | `eph-git` is the only crate that may build a `git` command. Enforced by `disallowed_methods` on `std::process::Command::new` / `tokio::process::Command::new` outside an allowlist of crates, plus an `xtask` syntax-tree check that the allowlist's other entries never start `git`. |
| 5 | Append-only stays append-only | The log, the cost ledger and the Odeon archive are types that expose append and read only. No truncating open exists for them. |
| 6 | Secrets write-only, env-injected | `Secret` is a newtype with no `Debug`, `Display`, `Clone` or `Serialize`. `eph-api` has no variant that carries one outward. The only consumer is the spawn environment. |
| 7 | Every degradation visible | One degradation channel. `#[must_use]` results, and `clippy::let_underscore_must_use` denies silently discarding one. |
| 8 | Prompt text lives in `prompts/` | Unchanged: loaded from the harness home and seeded from the bundled `prompts/`. String literals sent to a model are refused by an `xtask` check. |
| 9 | Schema'd files carry `schemaVersion` | Every persisted type implements `Versioned` in `eph-schema`, validated on read and before write. |
| 10 | No new dependency without a memo | `xtask deps` fails when a workspace crate's direct dependency is not in `docs/RUST-CRATES.md`. `cargo deny` checks licenses and advisories over the whole graph. |
| 11 | Cost figures come from the durable ledger | Cost is readable only through the ledger crate's API, which reads SQLite. No in-memory counter type exists. |
| 12 | UI values come only from design tokens | `eph-ui::tokens` is the one module allowed to build a `Color32`, a stroke width or a spacing value from literals (`disallowed_methods` elsewhere). |
| 13 | Watched-source content is data | Unchanged in substance. Fetched and studied content is carried as `Untrusted<T>` with its provenance, and no API turns it into an instruction. |

### Mechanisms that change under accepted ADRs

| ADR | Decision kept | Mechanism, then → now |
|---|---|---|
| ADR-0002 (two data planes) | PTY bytes and hook events stay separate planes | Electron main/preload/renderer tiers → one process. The PTY plane is a byte channel to the terminal widget, and the event plane is the hook socket into `eph-core`. |
| ADR-0007 (voice seam) | Provider-agnostic STT/TTS/duplex seam, ElevenLabs first, OpenAI Realtime fallback | Vendor JS SDKs → HTTP and WebSocket clients in Rust (crates memo'd at R12) |
| ADR-0014 (own the spawn) | Spawn, never attach | node-pty → `portable-pty` (ConPTY on Windows, openpty on POSIX) |
| ADR-0036 (two engine kinds) | A native engine, `eph-agent`, the harness owns | A TypeScript build target run by system `node` → a Rust binary in this workspace, built when M9 resumes |

### Requirements whose wording names the old stack

FR-1.1 names node-pty and xterm.js, NFR-8 names the renderer's sandbox, `contextIsolation`
and typed IPC, and NFR-16 names TypeScript. The requirement behind each stands. The
proposed amended wording is in `docs/RUST-PLAN.md` §10, to be applied on the Architect's
approval.

### Other consequences

- The gate chain is replaced: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test`, `cargo deny check` and the `xtask` invariant checks, in a CI job on
  Linux and Windows. `main`'s required checks are TypeScript gates under a ruleset only
  the Architect can edit, so the TypeScript CI keeps running on the frozen tree until he
  switches the ruleset (`docs/RUST-PLAN.md` §6).
- The roughly 5,000 TypeScript tests are not ported mechanically. Each phase writes the
  Rust tests its exit criteria name, and the scenario suites (TEST-STRATEGY §3) are
  rebuilt as Rust integration tests that drive `ephesusd` over the control socket.
- The project's verification discipline carries over with Rust tools: `cargo-mutants`
  for the mutation round (GYM-008's rules, control included), `cargo-llvm-cov` for the
  coverage floors, `egui_kittest` for driving the window in tests.
- Highest risk: terminal fidelity. xterm.js was a mature emulator. The widget is
  Ephesus's own code on a maintained emulator core, which is why the plan builds it
  early (R4) rather than last.

## Parity

The Rust build reaches parity when every functional requirement the TypeScript build
satisfied at M8c is satisfied by the Rust build, demonstrated by its own tests and
scenario suites, and the SRS §6 criteria the TypeScript build met are met again. A
criterion the TypeScript build never met stays open, and parity does not claim it. The
phase-by-phase definition is `docs/RUST-PLAN.md` §5, R14.

## Prior art

Alacritty (`alacritty_terminal`) and WezTerm (`portable-pty`) are the emulator and PTY
layers this design adopts. Rerun is a large egui application in daily use. Zed showed a
Rust editor can host terminals and agents with a small footprint.

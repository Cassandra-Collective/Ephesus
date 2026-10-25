# Decision memo — the Rust rewrite's vetted crate set (Set A APPROVED 2026-10-19)

**Approved:** Set A and the license allowlist, as one set, by the Architect on 2026-10-19
(DECISIONS-LOG, same date). MPL-2.0 stays out. Set B is not approved. **Asked:** the
Architect's approval of Set A as one set (his decision of 2026-10-19: "Vetted set, one
memo"). **Under:** invariant 10, ADR-0041 §7. **Checked by:** `xtask
deps` (every direct dependency of a workspace crate must be in an approved set below)
and `cargo deny check` (licenses and advisories over the whole graph).

Every figure below was read from the crates.io API on 2026-10-19: the latest stable
version, its license expression and release date, recent downloads (90 days), and the
declared MSRV. The workspace toolchain is Rust 1.96.0, so every MSRV below is met.

## Set A — asked for approval now

A version below is a caret requirement (`serde = "1.0"` admits 1.x). A major bump is a
new memo row.

### Runtime

| Crate | Version | License | Role | Phase | Why not `std` or our own code |
|---|---|---|---|---|---|
| `serde` (+ `derive`) | 1.0 (1.0.229, 2026-07-18) | MIT OR Apache-2.0 | Typed models for every persisted file and wire message | R1 | The ecosystem's serialization contract. Hand-written parsers are where schema drift hides. |
| `serde_json` | 1.0 (1.0.154, 2026-10-11) | MIT OR Apache-2.0 | JSON files, hook payloads, the control protocol | R1 | `std` has no JSON. |
| `thiserror` | 2.0 (2.0.21, 2026-09-23) | MIT OR Apache-2.0 | Typed error enums whose messages teach the rule | R1 | Derive-only, no runtime cost. The alternative is hand-written `Display` impls. |
| `jiff` | 0.2 (0.2.38, 2026-10-06) | Unlicense OR MIT | UTC timestamps (RFC 3339), durations, the trigger clock | R1 | `std::time` has no calendar or formatting. Rejected: `time` 0.3, which is comparable, with a smaller API for durations and spans. |
| `uuid` (`v4`) | 1 (1.28.0, 2026-10-10) | Apache-2.0 OR MIT | Message, memo, gate and spawn-token ids | R1 | `std` has no RNG. |
| `tokio` | 1 (1.53.3, 2026-10-13) | MIT | The hook and control sockets (Unix sockets, Windows named pipes), child processes, timers | R3 | `std` has no named pipes and no async process I/O. One runtime for every socket keeps the one-harness-per-home probe and both endpoints alike. |
| `portable-pty` | 0.9 (0.9.0, 2025-02-11) | MIT | ConPTY on Windows, openpty on POSIX (ADR-0014's spawn) | R2 | WezTerm's PTY layer. Writing ConPTY bindings ourselves is an FFI crate and the riskiest code in the system. |
| `notify` | 8 (8.2.0, 2025-08-03) | CC0-1.0 | Watching agent outboxes (Hermes) | R5 | Per-OS file-event APIs are FFI. Polling is the fallback, and its use is reported as a degradation. |
| `rusqlite` (`bundled`) | 0.40 (0.40.2, 2026-08-08) | MIT | The cost ledger, app-local state, the FTS5 keyword rung | R6, R8 | SQLite is the store ADR-0001 already chose (`better-sqlite3`). `bundled` builds SQLite from source with FTS5, with no system library to match. |
| `egui` | 0.37 (0.37.0, 2026-10-15) | MIT OR Apache-2.0 | The window's widgets, layout and painter (ADR-0041 §2) | R4 | The Architect's choice of framework. |
| `eframe` | 0.37 (0.37.0, 2026-10-15) | MIT OR Apache-2.0 | egui's native shell: the window, event loop and renderer backend | R4 | egui's own shell. The backend (glow, which is the default, or wgpu) is decided at R4 by measured memory. |
| `alacritty_terminal` | 0.26 (0.26.0, 2026-04-06) | Apache-2.0 | The terminal emulator core under the terminal widget | R4 | A VT emulator is a large state machine we would otherwise write. Its `tty` module is not used: PTYs belong to `eph-pty`. |

### Tests and tooling (never shipped)

| Crate | Version | License | Role | Phase |
|---|---|---|---|---|
| `tempfile` | 3 (3.27.0, 2026-03-11) | MIT OR Apache-2.0 | Temporary homes and checkouts in tests (dev-dependency only) | R1 |
| `egui_kittest` | 0.37 (0.37.0, 2026-10-15) | MIT OR Apache-2.0 | Driving the window headlessly in tests: query by label, click, type, snapshot | R4 |
| `syn` (`full`, `visit`) | 3 (3.0.9, 2026-10-12) | MIT OR Apache-2.0 | `xtask`'s syntax-tree invariant checks, so a wrapped call cannot slip past a line regex | R0 |
| `cargo_metadata` | 0.23 (0.23.1, 2025-11-12) | MIT | `xtask`'s dependency-rule and `deps` checks | R0 |
| `toml` | 1 (1.1.8, 2026-10-09) | MIT OR Apache-2.0 | `xtask` reads each crate's `Cargo.toml` to check its `[lints]` opt-in, which `cargo metadata` does not report. **Added by its own memo row, approved 2026-10-19** (DECISIONS-LOG, R0.2) | R0 |
| `proc-macro2` (`span-locations`) | 1 (1.0.107, 2026-07-19) | MIT OR Apache-2.0 | `xtask`: turns on source positions in `syn` spans, so an invariant refusal names its file and line (GYM-013 metric 4). Already in the graph under `syn`. **Added by its own memo row, approved 2026-10-19** (DECISIONS-LOG, R0.2) | R0 |

### Cargo tools (installed binaries, not dependencies)

| Tool | Version | License | Replaces |
|---|---|---|---|
| `cargo-deny` | 0.20.2 (2026-07-09) | MIT OR Apache-2.0 | `npm audit`, and license review by hand |
| `cargo-llvm-cov` | 0.9.1 (2026-09-06) | Apache-2.0 OR MIT | `@vitest/coverage-v8` and the coverage floors |
| `cargo-mutants` | 27.1.0 (2026-06-02) | MIT | `scripts/mutate.cjs` (GYM-008's rules carry over: a control, a clean baseline, no measuring during a round) |

`rustfmt` and `clippy` ship with the toolchain.

## The license allowlist for `cargo deny`

Asked with Set A: `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`,
`BSD-3-Clause`, `ISC`, `Zlib`, `CC0-1.0`, `Unlicense`, `0BSD`, `BSL-1.0`,
`Unicode-3.0`. Anything else in the transitive graph fails the gate by name and comes
back to the Architect. That includes MPL-2.0, a weak copyleft, which is left out until
he rules on it.

## Set B — expected later, NOT approved by being listed

Each gets its own memo at its phase, as PHASE-9-PLAN §10 did for Phase 9:

| Candidate | Phase | For |
|---|---|---|
| `keyring` 4.x | R6 | The secret broker's OS keychain seam (ADR-0010) |
| `windows-sys` 0.61 | R2/R3 if needed | A named FFI boundary crate (job objects, a pipe ACL), the one place `unsafe` would be allowed |
| an HTTP client (`ureq` or `reqwest`) | R10, R12 | GitHub App tokens, voice providers |
| `tokio-tungstenite` | R12 | OpenAI Realtime's WebSocket |
| `cpal` | R12 | Audio capture and playback |
| `sha2` | R10/R11 | Content hashes (bench, provenance) |

## Rejected

- **A memo per crate, every time.** The Architect chose a set.
- **A blanket allowance for any permissive crate.** Weakest control, and invariant 10
  would become a record of what happened rather than a gate.
- **`cargo deny`'s `bans.allow` as the vetted set.** It lists the whole transitive graph
  (hundreds of crates under egui), so approving it would mean approving crates nobody
  chose. `xtask deps` holds the direct dependencies, and `cargo deny` holds the graph to
  the license list and the advisory database.

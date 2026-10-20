# Ephesus — project memory for Claude Code sessions

Ephesus is a multi-agent harness desktop app: a "company of agents" governed by a human
Architect. It was built in Electron + React + TypeScript + Pixi.js + xterm.js + node-pty,
and **is being rewritten in Rust, window included (ADR-0041, 2026-10-19)**. The
TypeScript tree is frozen as the behavioural reference until parity. **The
company's primary standing mission is to improve itself** — every session here either
builds the system or improves how it is built, and both flow through the same
documentation and gates.

## What kind of session is this?

- **Implementing the system** → follow `BUILD-PROMPT.md` exactly. It defines your role,
  reading protocol, work packages, and report format. Resume from the *Resume here*
  block at the top of `docs/PROGRESS.md` — it names the next step and what waits on the
  Architect. As of 2026-10-19 that is the Rust rewrite, `docs/RUST-PLAN.md` (R0 → R14),
  which runs before M9. M9 is still planned and not authorised to start.
- **Improving the system/process** → follow the Gymnasium loop:
  `docs/adr/ADR-0015-gymnasium-self-improvement.md`. Improvement work is proposed,
  gated, and measured — never freelanced.
- **Editing documentation** → respect precedence (below) and cross-reference rules:
  a requirement lives in one place and is linked from everywhere else, never restated.

## Source of truth and precedence

Read `README.md` first for the subsystem map (Artemis, Hermes, Agora, Library, Odeon,
Herald, Harbor, Watch, Terraces, Gymnasium). When documents seem to disagree:
**SDD > ADR > SRS > README** for *how*; **SRS > SDD** for *what*. ADRs are append-only —
never edit an accepted ADR; supersede it.

| Need | File |
|---|---|
| Requirements, use cases, acceptance tests | `docs/srs/SRS.md` |
| Why decisions were made | `docs/adr/` |
| Architecture, data models, IPC, sequences | `docs/sdd/SDD.md` |
| Coding/security rules, Definition of Done | `docs/ENGINEERING-STANDARDS.md` |
| Trust boundaries, what each control stops, residual risks | `docs/THREAT-MODEL.md` |
| What tests are owed | `docs/TEST-STRATEGY.md` |
| Build order and current state | `docs/IMPLEMENTATION.md` + `docs/PROGRESS.md` (its *Resume here* block; closed milestones in `docs/progress/`) |
| Claude Code automation in this repo | `docs/AUTOMATION.md` |

## Commands

The Rust workspace (from R0.1; set `CARGO_TARGET_DIR` outside OneDrive first):

```bash
cargo xtask gate   # fmt, clippy -D warnings, test, deny, invariants, deps, coverage (RUST-PLAN §4)
```

The frozen TypeScript tree (its CI still guards `main` until R14):

```bash
npm run dev        # Electron app with hot reload
npm run typecheck  # node + preload + web projects; must be green at every commit
npm run lint       # ESLint + Prettier, zero warnings
npm test           # Vitest
```

## Non-negotiable invariants (digest — full list in BUILD-PROMPT.md §3)

Strict TS, no `any` past validators · renderer never touches Node/fs · atomic writes
(temp+rename) for shared files · only main commits to the Agora · append-only stays
append-only · secrets write-only, env-injected · every degradation visible in UI ·
prompt text lives in `prompts/`, never in code · schema'd files carry `schemaVersion` ·
no new dependencies without a decision memo · UI values come only from design tokens.

## Conventions

Conventional Commits, subject ≤ 72 chars. Branches `feature/<topic>`, `fix/<topic>`,
`agent/<name>/<topic>`. Every PR carries evidence (screenshot / terminal capture /
test output). One work package per PR. Do not create PRs unless asked.

## Project skills available here

`/ephesus-engineer` (**the operating protocol the others are executed to**: when to
consult the Architect, the invariants, the evidence tiers, honest verification
reporting) · `/goal` (drive the current milestone run: authorship + branch policy +
package loop) · `/build-package` (execute next BUILD-PROMPT work package) ·
`/milestone-review` (verify exit criteria, update PROGRESS) · `/doc-sync`
(code↔docs drift check) · `/improve` (file a Gymnasium improvement proposal) ·
`/research` (run a Stoa research cycle over the watchlist).
See `docs/AUTOMATION.md`.

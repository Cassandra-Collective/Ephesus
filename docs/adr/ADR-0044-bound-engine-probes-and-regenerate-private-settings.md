# ADR-0044 — Bound engine probes and regenerate private settings

**Status** accepted · **Date** 2026-11-24 · **Scope** the Rust build

## Context

The reference adapter declares both version and authentication probes. RUST-PLAN §3.2
named the version probe alone. Both commands need a bounded process lifecycle,
including descendants that hold output pipes after the root exits. The frozen
implementation gives each probe ten seconds. [ADR-0042](./ADR-0042-owned-process-group-shutdown.md)
places native lifecycle operations in `eph-process`.

[ADR-0026](./ADR-0026-engine-isolation-and-the-harness-as-sole-hook-author.md)
puts Claude's harness settings in an agent's private config directory and gives the
harness sole authorship of its hooks. The frozen merge can preserve foreign hooks and
permission grants the latest declaration withdrew. [ADR-0035](./ADR-0035-an-engine-prompt-is-declared-in-advance.md)
records the earlier limitation when the target file was shared.

## Decision

The Architect approved both decisions on 2026-11-24 (DECISIONS-LOG, R2.2 decisions
before code).

- `eph-engines` constructs and runs version and authentication probes. A probe uses
  the ten-second deadline, separate stdout and stderr, and the existing owned native
  process boundary. An authentication probe uses the same engine config and
  credential lookup as the spawn. Start failure, completion, and absence of a
  recognized answer remain distinguishable. RUST-PLAN §3.2 includes both probe kinds.
- On each launch, the harness regenerates `eph-settings.json` from the current
  declarations and retains that private generated file after exit for inspection.
  A damaged or linked existing settings file is refused. The next launch takes its
  hooks and grants from its current declarations, so withdrawal takes effect on that
  launch. The engine's own `.claude.json` still follows SDD-RUST §3.2's onboarding
  and trust rules.

## Options considered

- Extract a shared probe runner with `eph-git`. This would change Git's tested
  deadline path in the engine package.
- Keep the process rule limited to version probes. The reference authentication
  probe would still need an execution boundary before R2.3 could call it.
- Remove generated settings on clean exit. Retaining the last generation gives the
  Architect the exact settings from that launch to inspect.
- Preserve the frozen merge and restoration. That retains the hook and permission
  carryover that motivated this decision.

## Consequences

The CLI's actual arguments, cwd and environment remain explicit. Ownership keeps the
limits of ADR-0042 on each platform. A probe supplies evidence for its own
invocation. Conformance replays pinned captures, and R2's exit separately exercises
the installed, signed-in reference engine under a PTY. The target checkout receives
no settings writes. The TypeScript reference retains its existing behavior.

# R2.3 — Agent lifecycle and terminal commands

**Status** verification in progress · **Branch** `feature/r2-3-eph-agents`

## Decisions

The Architect approved SDD-RUST §3.4, decisions 1A–5A, A1–C1, and D1–E1
on 2026-11-25. The [decision log](../DECISIONS-LOG.md),
[ADR-0045](../adr/ADR-0045-own-agent-launches-through-teardown.md), and
[ADR-0046](../adr/ADR-0046-queue-terminal-input-and-retry-cleanup.md)
record the approvals and alternatives. The [Rust plan](../RUST-PLAN.md)
records the general-harness direction for wrapped CLI and native engines.

## Implementation

- **Tier C.** `eph-agents` owns the launch, installer consent, typed
  lifecycle, queue, per-hire cleanup reservation, and respawn deadlines.
  The caller drives the event loop. The owner admits a PTY in its current
  turn. Probes, durable records, and worktree operations run off that
  turn. A failed revoke or cleanup start preserves the reservation
  until the same generation can retry it.
- **Tier C.** The manager carries the executable selected by the final
  version probe into private authentication and native launch. The
  installer receives the base environment after consent. Broker
  values and the hook registration enter only the subsequent agent
  launch. The PTY output path masks their values.
- **Tier C.** The terminal input worker has one outstanding write.
  Operator text arms the first CR after acknowledgement. Escape clears
  held text, scheduled keys, and the right of an outstanding text write
  to arm a key. A write already dispatched to the terminal can finish.
- **Tier C.** The existing Agora writer appends typed lifecycle records,
  correlated by the acknowledged request sequence. A launch publishes
  its running state after the required record succeeds. Task return
  keeps the assignee and moves only in-progress work. Cleanup continues
  when recording exit or unwind fails.

## Local evidence so far

- **Tier A.** The Windows headless fixtures exercised normal launch,
  explicit installation, process and worktree cleanup, a durable stop
  during preparation, and retry of a failed revoke while another hire
  continued. Git for Windows initially refused a verbatim path in
  `git worktree add`. A Windows regression now exercises creation and
  removal with that path.
- **Tier A.** The WSL2 workspace suite ran on Rust 1.96.0 and Git 2.53.0.
  Nine headless cases passed in the latest workspace run. The control
  filter selected zero headless cases.
- **Tier A.** The local Windows and WSL2 coverage reports passed every
  existing crate floor after the targeted fixtures. Each reported
  `eph-agents: has no floor yet`. These reports precede the final
  committed-tree proof and coverage ratchet.

## Owed evidence

Mutation verdicts for every changed crate, exact-tree repeated Windows and
WSL2 tests, the seeded coverage floor and final gates, approved publication,
and three-platform CI are still owed. The fixture uses real PTYs and local
test executables. The signed-in reference-engine PTY run and effective grant
checks are owed at R2 exit.

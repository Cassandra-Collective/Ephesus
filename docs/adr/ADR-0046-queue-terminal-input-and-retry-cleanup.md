# ADR-0046 — Queue terminal input and retry cleanup by generation

**Status** accepted · **Date** 2026-11-25 · **Scope** the Rust agent manager

## Context

The agent manager owns terminal input and drains a bounded output queue.
A synchronous input write can wait on a child that is itself waiting for
the manager to drain its output. Cleanup can also fail after the process
has ended, while the hire's token or worktree is still reserved.

## Decision

The Architect approved both changes on 2026-11-25 (DECISIONS-LOG, R2.3
review decisions).

- The manager uses a PTY input worker with one outstanding write per
  terminal. The owner continues draining output and driving other hires.
  It schedules the first submit key after the worker acknowledges every
  byte of the operator's text. Escape cancels pending submit keys and
  disarms a text acknowledgement that has yet to arrive.
- Failed revocation or a cleanup effect that could not start keeps the
  hire reserved. The caller can retry the exact generation after its
  active terminal and worker settle. The outcome retains the reason until
  cleanup finishes.

## Options considered

Synchronous input would retain the cross-hire backpressure risk.
Automatic cleanup retries would need their own attempt and timing policy.

## Consequences

The caller drives the pending input and cleanup observations through
the manager. Existing direct PTY callers can still request synchronous
input. Terminal shutdown and output draining retain the ownership rules
in [ADR-0042](./ADR-0042-owned-process-group-shutdown.md) and
[ADR-0045](./ADR-0045-own-agent-launches-through-teardown.md).

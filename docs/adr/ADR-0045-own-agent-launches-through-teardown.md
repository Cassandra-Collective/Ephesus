# ADR-0045 — Own agent launches through teardown

**Status** accepted · **Date** 2026-11-25 · **Scope** the Rust build

## Context

A hire spans an engine probe, an optional installer, a workspace, a private
configuration and an owned terminal. Termination can remain unconfirmed, and
the same worktree path can be reused on respawn. A callback for the old launch
must never change the new launch or release its checkout. The frozen manager
supplies the lifecycle reference. [SDD-RUST §3.4](../sdd/SDD-RUST.md#34-agent-manager-r23)
defines the Rust contract.

## Decision

The Architect approved the manager design and its dependencies on 2026-11-25
(DECISIONS-LOG, R2.3 design approval).

- `eph-agents` owns the lifecycle and deadlines for each hire. Its caller drives
  events. Results name their launch, and PTY events also name their session.
  Launch admission and PTY acquisition happen during one owner turn.
  The hire's resources stay reserved through outstanding work and teardown.
  An unconfirmed shutdown holds that reservation while other hires continue.
- Only confirmed PATH absence can offer an install. Consent is bound to the
  displayed command and one launch attempt. The installer uses the hire's
  visible terminal and starts from a harness-owned directory. A successful
  exit, fresh version check and private authentication check precede the
  engine. An isolated worktree is allocated after the install stage.
- A required roster or lifecycle record failure refuses launch. A failed
  acknowledgement after PTY acquisition triggers termination and unwind.
  Exit record failures remain visible and cleanup continues. The existing
  Agora book holds typed lifecycle records correlated by launch request.
- Escape clears both held input and scheduled submit keys before sending the
  adapter's interrupt sequence. Current hook observations can confirm a
  submit and cancel its retries.
- Validate effective grants at the manager boundary and measure the reference
  engine's restrictions in a signed-in disposable run under the existing
  threat model. Agora administrative metadata stays outside the grant.

## Options considered

A dedicated worker as the public owner and a new async runtime were considered.
The caller-driven design uses the existing workspace crates. Transferable
leases could overlap cleanup and relaunch at the cost of tracking ownership
across replacement attempts. Preserving the frozen Escape behavior would
allow a submit key to reach the terminal after an interrupt.

## Consequences

Cleanup can defer a replacement for one hire. The caller drives the other
hires and makes the outstanding resource and its reason visible. Engine
support runs through the adapter contract for wrapped and native engines.
Hook delivery and the terminal view arrive in their planned phases.

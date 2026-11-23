# ADR-0042 — Own the shutdown boundary at spawn

**Status:** accepted · **Date:** 2026-11-16

## Context

[ADR-0014](./ADR-0014-owned-spawn-and-floor.md) requires Ephesus to own
agent processes. The [R2.1 probe](../implementations/2026-11-16-r2-1-pty-tree-probe.md)
showed both descendants survived the direct `portable-pty` kill in every
Windows trial. On Linux both survived in every trial with hangup ignored
or a separate process group. The Git deadline path has the same requirement
to account for the processes it starts (R1.3).

## Decision

- Windows launches the root into a Job Object at process creation. The job
  holds descendants without breakaway and terminates them on the last
  handle closing. Job setup failure fails the launch. Verify explicit
  termination as well as close, including immediate descendant creation.
- Linux and macOS launch into a dedicated process group. Forced shutdown
  targets the owned group, including members that ignore `SIGHUP`. A
  descendant that changes groups is outside the guarantee. The returned
  result and documentation state that scope.
- `eph-process` is the named FFI boundary under
  [ADR-0041](./ADR-0041-ephesus-is-rewritten-in-rust.md). It owns the native
  operations through the approved `windows-sys` 0.61 and `libc` 0.2
  dependencies. Its callers own the program and arguments. Only `eph-git`
  constructs commands for Git. The Windows boundary must arrange the Job
  Object and ConPTY attributes in the same native launch.

The terminal plane keeps the plan's exact environment and the byte stream
contract in FR-1.1, subject to ADR-0010 redaction. R2.1 includes regression
coverage for secrets split where one registered value prefixes another.

## Options considered

- Direct `portable-pty` kill leaves live descendants in both measured hosts.
- Windows assignment after spawn leaves a window for the first descendant
  to start outside the job. Assignment at creation closes that window.
- Linux containment across different process groups was considered. R2.1
  adopts the process-group contract across Linux and macOS and reports its
  scope.

## Consequences

The R2.1 implementation must prove the process ownership path with real
subprocesses and use the same boundary for the R1.3 Git deadline path.
POSIX process-group shutdown has an explicit limit. The probe has Windows
and Linux evidence. The implementation owes macOS coverage through the
workspace's normal CI matrix.

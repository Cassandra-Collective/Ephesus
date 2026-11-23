# ADR-0043 — Preserve the terminal spawn plan

**Status:** accepted · **Date:** 2026-11-16

## Context

[ADR-0042](./ADR-0042-owned-process-group-shutdown.md) requires the exact spawn
environment. Portable-PTY 0.9.0 supplies `SHELL` and substitutes a home directory
when the requested working directory is invalid. The frozen Windows terminal
can run an unrecognized batch shim. A running child still holds credentials
that the broker has since rotated.

## Decision

- On Linux and macOS, implement native `openpty` within the existing
  `eph-process` boundary, using the approved `libc` dependency. The caller's
  exact environment and working directory reach the child. An invalid
  working directory fails launch.
- Keep the Windows batch fallback for `.cmd` and `.bat` shims whose executable
  target cannot be identified. Invoke the system command interpreter inside
  the same Job Object. Check argument encoding and refuse inputs the launcher
  cannot represent safely. Ordinary executables use the native argument path.
- Until the session ends, redact both its initial broker values and the
  currently supplied values. This covers credentials the child still holds
  after a broker rotation. The retained values are memory-only.

## Options considered

The Architect considered retaining the library's POSIX environment defaults,
requiring an executable for every Windows launch, and redacting only current
values. He approved the exact-plan and compatibility behavior above.

## Consequences

The portable-PTY reproducer remains useful for the original failure. The
production terminal uses platform PTY APIs through the named boundary.
The owned shutdown guarantees remain those of ADR-0042.

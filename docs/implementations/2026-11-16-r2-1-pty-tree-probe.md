# R2.1 process-tree shutdown probe

The [R2 decision](../DECISIONS-LOG.md) calls for measuring `portable-pty` before
choosing how to tear down an agent's process tree. The probe also informs the Git
deadline path in R1.3, where the runner can return before the Git tree has exited.

## Reproduce

In the repository root, set `CARGO_TARGET_DIR` to a directory outside OneDrive,
then run the standalone probe with Rust 1.96.0.

```sh
cargo +1.96.0 run --locked --manifest-path scripts/probes/r2-1-pty-tree/Cargo.toml --example tree-kill
```

The fixture spawns itself in the PTY and starts two generations of descendants.
Each adds a byte to a private file about every 40 ms. The driver
first verifies that all three files grow. It then calls `ChildKiller::kill()`,
observes the root's exit status, and counts new bytes for one second with the
terminal still open. A positive post-exit count proves that process survived.
The workers exit when the temporary directory is removed, with a 15-second
lifetime as a fallback.

The Unix runs compare descendants that share the root's group with default
signal handling, share it but ignore `SIGHUP`, and start in a separate group.

`portable-pty` 0.9.0 requests cursor inheritance on Windows. A headless caller
must answer the ConPTY cursor-position query during setup. The probe replies
with row 1, column 1 and drains the output while it measures the processes.
Before adding the reply, the Windows fixture timed out at startup with zero
heartbeats. [Microsoft's ConPTY documentation](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)
describes the asynchronous exchange.

## Windows results

Windows x86-64, build 26300, Rust 1.96.0, `portable-pty` 0.9.0. The command
exited 0 and the standalone example passed Clippy with warnings denied.

| Trial | Before kill (root, child, grandchild) | Root exit code | After root exit (root, child, grandchild) |
| --- | --- | --- | --- |
| 1 | 25, 24, 24 | 1 | 0, 25, 25 |
| 2 | 25, 25, 25 | 1 | 0, 24, 24 |
| 3 | 25, 24, 24 | 1 | 0, 25, 25 |

Windows process-tree shutdown needs more than this call. Closing the
pseudoconsole can affect attached clients and is outside this measurement.

## Linux results

Linux 6.8.12 in a disposable 32-bit x86 guest under v86 0.5.473, cross-built
with Rust 1.96.0 for `i686-unknown-linux-musl`. The guest command exited 0
and Clippy passed for this target. Each fixture process produced 21–24
heartbeats during the one-second control. The root exited from `SIGHUP`
in every trial.

| Descendant layout | Post-exit counts (root, child, grandchild), trial 1 | Trial 2 | Trial 3 |
| --- | --- | --- | --- |
| Shared group, default signals | 0, 0, 0 | 0, 0, 0 | 0, 0, 0 |
| Shared group, ignores `SIGHUP` | 0, 22, 22 | 0, 22, 22 | 0, 23, 23 |
| Separate process group | 0, 22, 23 | 0, 21, 21 | 0, 23, 22 |

Both descendants survived in all six trials that changed signal disposition or
process group. The shared-group control saw no post-exit heartbeat during the
sampling window. The probe covers the Windows host and this Linux guest.

## Proposed Windows dependency decision

The Architect approved this path in [ADR-0042](../adr/ADR-0042-owned-process-group-shutdown.md).

Approve `windows-sys` 0.61 for a named FFI boundary that owns a Windows Job
Object. Create a non-inheritable job handle, configure
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` without breakaway, and associate the root
at process creation so its first instructions already run in the job. Exercise
`PROC_THREAD_ATTRIBUTE_JOB_LIST` together with ConPTY and use the same job
ownership contract for the Git deadline path. Verify both explicit termination
and last-handle close against the real fixture, including a child that starts
its own descendant immediately. Return a typed error if job creation or
assignment fails. Test nested-job compatibility on the supported Windows CI
host before treating the path as established.

Adding a process to a job after `portable-pty` returns its running child
would leave a window in which the child could spawn outside the job. The
dependency and spawn integration are proposals.
[Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [process creation attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute).

## Proposed Linux dependency decision

The Architect approved the dedicated process group with its stated limit in
[ADR-0042](../adr/ADR-0042-owned-process-group-shutdown.md).

Approve `libc` for the named FFI boundary if the shutdown contract covers
processes in the launched group. Keep the launch-time group identity and use a
group-directed `SIGKILL` for forced shutdown. This reaches members that ignore
`SIGHUP`. The separate-group fixture demonstrates the additional scope a
group-directed operation cannot cover. If the contract includes those
descendants, evaluate a delegated cgroup v2 instead and exercise that fixture
before claiming coverage. Both designs still require implementation and
verification. [Linux signals](https://man7.org/linux/man-pages/man2/kill.2.html)
and [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html).

## Pre-integration evidence

- The frozen redactor emitted one mask for `demo-tail` in one chunk and a mask
  followed by `-tail` when split after `demo`, with both values registered.
  The Rust filter masks the full value at every split. It preserves ordinary
  bytes, including non-UTF-8 output, and keeps completed mask marks across
  credential rotation.
- Windows x86-64 build 26300, Rust 1.96.0:
  `cargo test --locked --offline -p eph-process -p eph-pty --lib`
  passed 4 and 12 tests. The native piped and ConPTY paths both placed three
  real processes in their Job Object. Explicit termination reached zero
  active members and a signaled root handle. The job count alone could reach
  zero before the root handle signaled in the initial run.
- Linux 6.8.12 x86 guest, Rust 1.96.0, target `i686-unknown-linux-musl`:
  `cargo test --locked --offline --target i686-unknown-linux-musl -p eph-process -p eph-pty --lib --no-run`
  produced test executables that passed 2 and 12 tests in the guest. The
  native group test started three processes that ignored hangup and verified
  a killed root and EOF on the pipe those processes held open.
- The Windows `cargo xtask gate` baseline at the R1 handoff stopped in the
  Git test suite. `a_git_past_its_deadline_is_stopped_and_says_so` hit error 32
  on test directory cleanup. The isolated rerun failed at the same line.
  This baseline was taken before the production Git integration.
- `cargo test --locked --offline -p xtask --lib --quiet` passed all 120 unit tests.
  `cargo xtask invariants` and `cargo xtask deps` passed over 7 crates and 52
  source files. The named FFI boundary's lint exception accepts the workspace
  table with just the unsafe-code level changed to allow explicit expectations.

## Integrated implementation evidence

- Windows x86-64 build 26300, Rust 1.96.0. The plain and coverage-instrumented
  workspace suites passed, including 110 Git, nine native process, and
  19 PTY tests. Real child fixtures checked ConPTY input, interrupt bytes,
  the reported 97×29 resize, redaction after credential rotation, and the
  exit and output EOF. Native tests also confirmed job termination through
  last-handle close, without an explicit termination request. The real Git
  runner stopped an already-started descendant holding output after the
  root exited. The control ran the same Git alias outside the runner and
  observed both its start marker and its delayed write.
- The coverage-instrumented PTY fixture initially failed its exact-environment
  assertion. Launching the instrumented binary directly proved that the
  Windows LLVM runtime adds `__LLVM_PROFILE_RT_INIT_ONCE` inside the child.
  The fixture accounts for that marker and passes the coverage runner's
  profile path explicitly when present. Production spawn adds no values.
- Linux 6.8.12 x86 guest. Real PTY and process suites passed 18 and four
  tests. Git 2.49.1 passed all 21 Git runner tests, including both deadline
  cases and late output on each pipe. The broader Git suite exceeded the
  guest's allotted time. Clippy passed for `i686-unknown-linux-musl` and
  `aarch64-apple-darwin` across the three affected crates.
- The Windows Rust gate passed formatting, Clippy, tests, dependency
  screening, invariants and the dependency allowlist. Existing coverage
  floors passed. The new crate floors require measurements from the CI
  matrix under ENGINEERING-STANDARDS §1.1. The five frozen TypeScript
  checks passed.
- `cargo xtask mutants xtask` returned `ROUND OK`. It caught all 496
  viable mutants of 532 with zero timeouts. The negative control missed
  all 123 viable mutants, and the command verified that the sources
  were unchanged across the round.
- Independent review verified that the narrowed shim recognizer matches the
  installed npm `claude.cmd` and found an existing executable at its target.
  The native tests cover an unrecognized batch file with spaces and an
  argument refused by the checked interpreter path.

POSIX forced shutdown covers members of the dedicated group. Descendants
in another group can survive. The macOS runtime and the two initial
coverage floors still need the CI matrix.

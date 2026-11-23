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
  assertion. Launching the instrumented binary directly on Windows proved that
  LLVM adds `__LLVM_PROFILE_RT_INIT_ONCE` inside the child. Linux CI observed the
  same marker, so the fixture allows it on both hosts. The fixture passes the
  coverage runner's profile path explicitly when present. Production spawn
  adds no values. The Linux guest fixture accepted the exact marker and
  deliberately failed when given an additional unrelated environment key.
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

## First CI run and corrections

The [first evidence run on 33250b4](https://github.com/Cassandra-Collective/Ephesus/actions/runs/43949500607)
passed docs, attribution and the frozen TypeScript suite. Its three native
jobs exposed distinct failures.

- Linux passed the plain Rust suite. Coverage failed when LLVM inserted
  its initialization marker into the two exact-environment fixtures.
- Windows passed the plain Rust suite. The instrumented random-kill test
  left `.git/HEAD.lock` and the reopened committer could not reconcile.
  In a disposable local Git repository, either this lock or
  `.git/refs/heads/main.lock` returned exit 128 on commit. Removing it
  restored exit 0. The Architect approved recovering only those known
  locks under the committer lock at open and reporting the result.
- macOS passed all 108 Git tests and the three-process forced-shutdown
  test. The PTY geometry fixture observed the new dimensions and then
  got `EPERM` while collecting the normal exit. Darwin excludes zombies
  from group signaling and can return this code when the group has no
  signalable member. Recovery now requires an observed root exit and a
  native group query returning exactly that root. Any group with another
  member or an inconclusive query keeps the original failure.
  [XNU group signaling](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c#L1612-L1621)
  · [XNU process listing](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c#L377-L454).

On the corrected tree, the local Windows Rust gate passed its first six
steps and every existing floor. The plain and instrumented runs each
passed 114 Git, 93 Agora, nine process and 19 PTY tests. The focused Git
mutation run caught all 11 viable ref-lock recovery mutants. The five
frozen TypeScript checks passed, and Apple ARM64 cross-Clippy passed for
all four affected crates. Native CI must still verify the corrected tree.

## Windows tree fixture follow-up

[CI on 8756e19](https://github.com/Cassandra-Collective/Ephesus/actions/runs/44046337806)
produced three measurements each on Linux and macOS. The first Windows
attempt produced a measurement. Attempt 2 timed out in the plain Windows
piped tree tests. Attempt 3 timed out in the instrumented last-handle-close
test. Twelve concurrent local runs of the Windows process tests on one
allowed CPU passed. The CI timeout did not identify which stage stalled.

With ready markers and diagnostics added, the local piped tests reproduced
the exact-count failure with all three fixtures ready, a live root, and four
active members. A Windows process-tree trace observed `conhost.exe` under
the piped root in that reproduction.

Each fixture now reports its PID. The test checks that the IDs are distinct,
that the root matches the launcher's handle, and that each of the three
processes is live in the same Job Object. Startup failures report job
membership, root status, and child output. Explicit termination checks zero
active members and the root exit code. Last-handle close observes each
fixture process handle signal and EOF on both pipes. The ConPTY test also
requires its stream to close. Start and shutdown have independent bounded
waits. [IsProcessInJob](https://learn.microsoft.com/en-us/windows/win32/api/jobapi/nf-jobapi-isprocessinjob).

On the final fixture, the Windows gate passed the nine process tests in
both builds and reached coverage. Only the two new crate floors remain
unseeded. Twelve concurrent one-CPU runs of the nine tests passed in each
of the plain and instrumented builds.

## Git coverage variation

[Three attempts on b907852](https://github.com/Cassandra-Collective/Ephesus/actions/runs/44073093548)
produced all nine artifacts, and the native suites passed. The third macOS
attempt measured `eph-git` at 1,111 of 1,123 lines (98.93%). The other two
macOS attempts covered 1,113 lines (99.10%). The artifacts do not identify
which lines differed. The ratchet refused the batch against the existing
98.96% floor.

A Git test now blocks the commit-message write with a real directory at the
target. It verifies that the error is reported and leaves HEAD and the work
intact, then removes the obstruction and confirms the next commit lands the
work. The existing shutdown test now deterministically observes an
unconfirmed poll followed by confirmation. Ref-lock recovery shares one
error classification for a lock that vanishes before metadata inspection or
removal. The local Windows Git suite passed 115 tests. The Windows gate
measured `eph-git` at 99.64% and stopped only at the two unseeded floors.
Six focused Linux guest tests and Apple ARM64 cross-Clippy passed.
The focused ref-lock mutation round caught all ten viable changes.
The final sources need a new set of native measurements.

## Guard verification — 2026-11-20

The package audit found that the new native and PTY guards still owed
controlled mutation evidence. The initial rounds exposed missing assertions
for geometry limits, argument encoding, PATH and shim defenses, terminal
read errors and disposal. Regression tests now observe public-session resize
inside the child, exercise each invalid geometry axis, and refuse device
batch paths and malformed npm prefixes even when the executable exists.
Windows passed 15 native and 24 PTY tests; the Linux 6.8.12 i686 guest passed
eight native and 23 PTY tests, using Rust 1.96.0. The five frozen TypeScript
checks passed. These plain test results do not establish the final mutation
verdict or the macOS runtime result.

A real piped fixture joins a second fixture-owned group, and the test reads
its changed group identity before requesting shutdown of the original group.
The escaped root remains alive and is then explicitly killed and reaped.
Both fixture roots are cleaned up. This verifies ADR-0042's stated limit.
The error classification seam receives the actual native error in production;
deterministic ESRCH, EPERM and EINVAL cases verify that only an absent group
is accepted on Linux and other failures retain their OS error. The Darwin
permission exception additionally checks the actual root state and native
group membership. Apple's [libproc implementation](https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c)
confirms that `proc_listpgrppids` returns a PID count, after converting the
underlying byte count.

The Architect approved four equivalent flag-operation exclusions and the
separately documented observation limit for `Attributes::drop`; no other
survivor is excluded. The openpty descriptor slots now start at zero because
success overwrites them and failure returns before using them. Independent
review checked the precise exclusion matches and the macOS guard selector.
The new workflow runs both crates' tests for its 17 selected guard mutants,
then runs a no-tests control over one shard. It retains the reports and source
fingerprints. Its actual report judge accepted a healthy synthetic result
and refused nine invalid cases: failed or missing baseline, no viable mutant,
timeout, round survivor, false control kill, inconsistent counts, and either
command failing. The shell block and embedded Python judge passed syntax
checks. Native Darwin execution and the final Windows/Linux mutation rounds
remain pending, followed by three fresh CI measurements per platform.

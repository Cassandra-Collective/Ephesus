# R2.1 guard proof specification and evidence

This supplements the generated mutations in `.cargo/mutants.toml` and follows
the five certification rules in `xtask/src/mutants.rs`. R2.1 remains open until
the final source has certified guard results and the coverage ratchet passes.

## Aimed seam mutations

Run these in an isolated copy of the source commit containing this specification.
Keep the selected source hashes, command output and return status. Before any
plant, each package/filter combination must pass with at least one test run.
Plant one change at a time and restore the exact original bytes after each run.
A kill requires an actual failing-test result; build failures are unviable and
timeouts invalidate the claim. Repeat the same plants with the project's
`__ephesus_mutants_control_matches_no_test__` filter: every viable plant must
survive with zero tests run. Compare the original and restored source hashes.
Never collect coverage while the isolated sources contain a plant.

| Plant | Location | Required regression | Platform |
| --- | --- | --- | --- |
| Replace the recovery call with an empty result | `Committer::open_with` | `ref_lock` tests through the committer | Windows or POSIX |
| Remove `agora/ref-lock` from `BOOT_CHECKED`, adjusting its fixed array length | `agora.rs` | `a_boot_recovers_known_ref_locks_and_reconciles_the_interrupted_work` checks the following boot | Windows or POSIX |
| Report successful removal as failed | `Agora::open` | Same recovery test checks the successful-removal message | Windows or POSIX |
| Report failed removal as successful | `Agora::open` | `boot_reports_a_ref_lock_that_is_not_a_regular_file_and_keeps_it` | Windows or POSIX |
| Skip pending-root cleanup | `complete_and_reap` | `pending_or_failed_final_status_is_reaped_without_blocking_the_result` | POSIX |
| Schedule cleanup for an already collected root | `complete_and_reap` | Same test covers code and signal statuses | POSIX |
| Reap synchronously instead of starting the worker | `complete_and_reap` | Same test releases its gate before asserting, so the plant fails without hanging | POSIX |
| Start the worker without invoking its reap action | `complete_and_reap` | Same test requires the action to start and finish | POSIX |
| Omit the native reap call after closing a session | `Backend::dispose` | Public session Drop test requires the real fixture PID to disappear from `/proc`, accepting only NotFound | Linux |
| Reject a batch command of exactly 8,191 encoded units | `batch_arguments` | `batch_arguments_and_error_codes_preserve_the_checked_contract` | Windows |
| Validate geometry without applying the public resize | `Session::resize` | Public terminal parity test observes 101 by 27 cells inside the Rust fixture | Windows and POSIX |

The native `root_exited` constant mutations run against the bounded native
adapter test alone. The broader runner suite includes the preexisting one-hour
deadline clamp test, which turns an always-false observation into a harness
timeout. A timeout is not a kill. Other runner mutations retain the broader
runner test selection. Both selections require their own baseline and control.

## Results before the follow-up

The published source was `681043f`, with coverage fingerprint
`fnv1a64:34a92da27563aba8`. These results identify what the follow-up must close;
they do not certify its changed sources.

- [Native Darwin guard run](https://github.com/Cassandra-Collective/Ephesus/actions/runs/44794677062):
  17 caught, no survivors, unviable mutants or timeouts; control five of five
  missed; both baselines green and source fingerprints unchanged. Reports were
  retained as the run artifact.
- Windows native/PTY round: 200 total, 162 caught, nine missed, 29 unviable,
  no timeouts. Its control missed all 49 viable mutants of 50. Linux guest:
  144 total, 115 caught, ten missed, 19 unviable, no guest timeout markers.
  Its control missed all 35 viable mutants of 36. Both pairs preserved all 82
  fingerprinted files. Survivors required platform mapping and the follow-up
  tests; the raw rounds have survivor verdicts.
- Ref-lock helpers and the new Agora reporting guard: 13 total, 11 caught,
  two unviable, no survivors or timeouts. Control: three viable mutants of
  four, all missed. Both baselines passed and all 82 files were unchanged.
  This round used the isolated cleanup candidate; these production helper
  functions were unchanged from the published source.
- Initial Git runner rounds are diagnostic only. Windows recorded one
  harness timeout. Linux labelled an 85-second guest timeout as caught; the
  failing log contains no failing-test result. That outcome is a timeout,
  and cannot count as an assertion kill. The exact observation mutation
  needs the bounded native test described above.

## Follow-up behavior

The Architect approved native terminal observation behind `eph-process`'s
`test-support` feature after PowerShell timed out in CI and locally. The
existing Rust parity fixture now reports its actual geometry after input;
the public test requires 101 by 27 cells. This replaces the external geometry
fixture and its duplicate standalone test. Native observation stays inside
the FFI boundary. The feature is enabled by the PTY crate's dev-dependency.
The native and PTY tests passed 38 cases on Windows and 30 in the Linux guest.

Unix session cleanup always collects the root after closing its terminal
handles. A previously observed status is cached; a pending root is collected
on the existing background worker. The Linux public Drop test observes the
fixture's own PID before Drop and requires it to disappear afterwards, alongside
the existing output and credential assertions. Git's owned completion path
similarly tests pending and failed final observations, known exits, background
scheduling and preserved results through the production scheduler.

The additional Git tests check ownership when either output pipe is missing,
two cleanup-confirmation polls through the post-root grace, controlled inherited
environment values on Windows, and actual native adapter observations before
and after shutdown. Windows also tests an exited root with a live descendant.
Final mutation outcomes and final CI coverage measurements remain pending.

The corrected Windows gate passed formatting, Clippy, both test builds,
dependency checks and invariants. Its 119 Git tests passed, including both
native adapter states. Coverage stopped only at the two unseeded crate floors;
all existing floors passed and no applicable module was untested. The five
frozen TypeScript checks passed. These results establish the baseline for the
next controlled rounds, rather than their mutation verdict.

## Follow-up controlled evidence (2026-11-22)

These measurements used the `e067f72` sources, coverage fingerprint
`fnv1a64:18d898dc8e1ce150`. Each completed pair had green positive and
no-tests-control baselines, no timeout, and unchanged tracked Rust inputs.
The raw platform rounds retain their survivor verdicts; a cross-platform
mapping is not a claim that either raw round reported ROUND OK.

| Scope | Positive measurement | No-tests control |
| --- | --- | --- |
| Changed Git runner, Windows | 42 total: 25 assertion catches, one inactive Unix survivor, 16 mutation-caused compiler rejections | Nine viable of 11, all missed |
| Changed Git runner, Linux guest | 42 total: 20 assertion catches, five inactive Windows survivors, 17 mutation-caused compiler rejections | Nine viable of 11, all missed |
| Native root observation, Windows | Both constant mutations failed the bounded adapter test | One viable, missed |
| Native root observation, Linux guest | Both constant mutations failed the bounded adapter test | One viable, missed |
| Aimed Windows seams | All six plants caused failing-test results | All six viable plants missed |
| Aimed Linux seams | All six plants caused failing-test results | All six viable plants missed |

The Git measurement omits the root observation constants, which have their own
bounded test and control. Every Git compiler rejection was checked for a
mutation-caused Rust type or syntax error. All 82 tracked inputs matched after
the aimed runs, and their controls ran zero tests. The
ref-lock/Agora helper round (11 assertion catches and two compiler rejections;
three viable control mutations of four all missed) is retained only for its
unchanged functions and relevant tests. The published Darwin guard evidence
similarly applies to 17 mutations in unchanged shutdown guard functions.

### Windows native/PTY classification

The broad measurement recorded 210 mutants: 171 raw catches, eight survivors,
31 raw unviables and no timeout. Log review distinguished 168 failing-test
summaries from three native access-violation aborts. It distinguished 21
mutation-caused Rust compiler rejections from ten environmental LNK1104
executable-lock failures in `wide_terminated` and `launch_arguments`. Those ten
cases remain unmeasured until the dedicated encoding-return round produces
valid outcomes. The three abort observations remain separate from assertion
kills.

The control recorded 53 mutants: 52 viable mutations missed, one genuine
`Ordering: Default` compiler rejection, no caught mutation and no timeout.
Its baseline and all 52 viable scenarios ran zero tests. The controller and an
independent SHA-256 comparison confirmed all 82 inputs unchanged; both tool
exit statuses were 2, preserving the positive survivor verdict.

The eight survivors were the observer's Left/Top subtraction signs, Unix-only
environment-name equality, Unix-only EIO guards changed to true/false, and
the shared Session Drop, Backend disposal body and disposal condition.
The disposal condition is active on Windows; closing the last Job Object
handle can mask its explicit termination request. These shared cleanup
mutations require the Linux result.

### Observer regression

Real ConPTY observations started at zero-origin rectangles, so replacing
`Right - Left` or `Bottom - Top` with addition survived. The observer now passes
the actual SDK `srWindow` rectangle through a private cell-count helper.
Literal expectations cover a zero origin, a nonzero origin, and a one-cell
window. Both real native and public Session resize tests are retained to
check the complete SDK-to-observation call path.

| Generated plant | Required regression |
| --- | --- |
| Replace either window-origin subtraction with addition | Nonzero-origin and one-cell literal cell counts |
| Replace observer/helper tuple results or change inclusive-bound arithmetic | Literal cell counts plus real native and public PTY geometry |

The isolated correction passed 16 native and 23 PTY tests on Windows with
Rust 1.96.0, the mutants profile, one Cargo build job and two test threads.
The native observation and public resize tests both passed. Of the 82
tracked Rust inputs, only `crates/eph-process/src/windows.rs` differed from
`e067f72`. Its SHA-256 was
`C534231B4DBC0DBE68AD9C876CFD5AFB3F83E2962956305D6EA9F95F25653684`.

### Observer certification on the corrected source

The corrected fingerprint is `fnv1a64:f7b4cd7840aa1e7e`. An independent
calculation reproduced the previous fingerprint before calculating this one
from the gate's exact 64 inputs, including its nested-fixture exclusions.
The function mapping reconstructed 56 original scopes: 55 unchanged bodies
occur exactly once, and only `current_terminal_size` changed. Its new helper
preserves the same arithmetic, and existing native and public geometry test
bodies are unchanged. The new literal test adds no global state or I/O.

The fresh observer round selected `current_terminal_size` and `window_cells`.
Its baseline ran 16 native `eph-process` tests. All 16 mutations caused native
failing-test summaries, including both origin-sign changes. The separate
integration baseline passed 16 native and 23 PTY tests.
Its control baseline and all four viable control mutations ran zero tests;
all four were missed. There were no unviable mutations or timeouts. The
controller recorded exit statuses 0 and 2 and preserved all 82 tracked inputs.
This certifies the changed native observer scope; its public caller requires
the separate controlled plant below. The original broad round keeps its raw
survivor and failure classifications.

The Darwin artifact was audited at the same level. Its baseline ran nine
native tests. All 17 positive commands selected both packages, but each
stopped after a native failing-test result before the PTY binary executed.
The five controls executed both package binaries with zero tests. This is
valid native Darwin guard evidence, without a positive PTY mutation claim.

The public caller plant replaces `Ok(window_cells(info.srWindow))` with
`Ok((1, 1))`. It selects
`session::tests::real_terminal_delivers_input_resizes_and_redacts_through_the_public_session`
in `eph-pty` alone. Its required results are a one-test baseline, a failure
at the geometry assertion with the observed `1x1` value, the same viable
plant missed with zero tests, and restoration of all 82 inputs.

On the isolated `5c88a65` copy the public baseline and restored run each
passed one test. The plant printed `fixture-geometry=1x1` and failed at the
public geometry assertion in `session.rs:499`. The control baseline and
planted control each ran zero tests and returned success. All 82 tracked
inputs matched after restoration.

The default Windows workspace run failed three Git deadline fixture tests
under concurrent load. All 25 focused runner tests passed with one test
thread. On the corrected tree, `cargo xtask gate` with two test threads
passed formatting, Clippy, the plain and instrumented suites, dependency
checks and invariants. Both builds passed 119 Git, 16 native process and
23 PTY tests. Coverage emitted the corrected fingerprint and zero untested
modules. All existing floors passed. The only two coverage findings were the
unseeded `eph-process` and `eph-pty` floors. The five frozen TypeScript
checks passed.

The isolated Windows encoding-return round ran 15 mutations. Fourteen had
failing-test summaries and the empty `wide_terminated` return aborted with
`STATUS_ACCESS_VIOLATION`. Its four viable controls all missed. The empty
return was then tested against the existing NUL-path validation alone.
It failed that Rust assertion, and the same mutation missed in its one-case
control. Together the two scopes give assertion evidence for all ten prior
link failures. Both pairs had successful baselines, no timeouts, and all 82
source hashes restored.

The single `Attributes::as_ptr` default-pointer mutation reproduced
`STATUS_ACCESS_VIOLATION` during the native tests. The sharded control
selected zero cases, so it was discarded. The same mutation missed in
the unsharded zero-tests control. The baseline passed and all 82 source
hashes matched. The positive result is an observed native abort.

On Linux, the corrected source ran with Rust 1.96.0 for
`i686-unknown-linux-musl` in the Linux guest, using cargo-mutants 27.1.0.
The baseline passed eight native and 22 PTY tests. The positive round
finished 146 cases with 119 failing-test summaries, 19 mutation-caused
compiler rejections, eight survivors, and zero timeouts.

Seven Linux survivors are in Darwin-only guards and match catches in
the native macOS artifact. The remaining survivor is the Windows-only
PATHEXT filter, which has a Windows assertion failure. Linux caught
the shared Session Drop, backend dispose body and disposal condition
survivors from Windows in the public session Drop test. It also caught
the Unix-only environment-name equality and both EIO guard constants.
The two Windows geometry sign survivors are covered by the corrected
observer proof above.

The Linux control selected 37 cases. All 36 viable cases missed with zero
tests running. The remaining case was rejected by the Rust compiler.
Both baselines passed and there were no timeouts. The positive round and
control each returned exit 2. All 82 input hashes match before and after
and agree with the working tree. With the applicable-platform matches
above, the broad rounds retain their raw survivor results while each
surviving guard has its relevant test evidence.

### Native CI on the corrected source

[CI run 45010830867](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45010830867)
measured `5c88a65` with coverage fingerprint `fnv1a64:f7b4cd7840aa1e7e`.
Linux and Windows uploaded all three reports. macOS uploaded the first two.
All eight reports have the exact run, attempt, commit and source fingerprint,
use cargo-llvm-cov 0.9.1, and list zero untested modules. The available
measurements passed the existing floors. Windows passed the public resize
test in both builds on all three attempts.

[macOS attempt 3](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45010830867/job/137178294793)
passed the plain Git suite. The instrumented
`native_adapter_observes_a_ready_child_before_and_after_owned_shutdown`
test timed out waiting for pipe EOF at `runner.rs:1000`. The shutdown call
and root exit observation had already returned successfully. Runner cleanup
later reported one orphaned `sleep`. Its process group was not recorded.
The coverage run failed and emitted no macOS artifact for that attempt.

The fixture prints its readiness marker before starting `sleep`. Apple's
[group signal implementation](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_sig.c#L1612)
uses a [member snapshot](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_proc.c#L4058)
that is collected before signal delivery. A fork during delivery is compatible
with the observed timeout. The log does not establish that the surviving
process belonged to the owned group or establish when it joined.

The official ratchet returned exit 1 with `macos: 2 of the 3 CI runs a raise
needs`. The floors file's hash stayed unchanged. R2.1 remains open pending
resolution of the shutdown failure and a complete native coverage set.

### macOS diagnostic candidate

The diagnostic is confined to the failing test. It records Git's PID and
native PGID before the fixture starts `sleep`, and retains the unreaped root
while waiting for both pipes. If a pipe times out, it records the PID, PGID,
state and command name for members of that group, then sends a second group
signal through the existing owner and records whether the pipe closes.
The original timeout still fails the test even if the second signal works.
Native execution is required to distinguish a surviving member from an
unrelated orphan and to determine whether a second signal recovers the pipe.

Darwin-target Clippy and both shell syntax checks passed on the final
diagnostic revision. The Windows gate passed its plain and instrumented
suites with two test threads, then stopped at the two unseeded floors.
The plain stage preceded the final macOS-only readiness check. The
instrumented stage included it and emitted fingerprint
`fnv1a64:4ad973daec75238a` with zero untested modules. The five frozen
TypeScript checks passed.

A disposable i686 Linux guest ran Git 2.49.1 through the same alias launch
shape. The shell reported parent PID 31, which matched the launched Git PID
and PGID. The probe then modeled a stale membership snapshot by signaling
only the root and alias shell after an additional member had joined. That
member remained live in group 31 and held the pipe. A second group signal
closed the pipe while the exited root still pinned the group. The probe
passed one test and exited 0. This checks the alias assumption and
demonstrates the mechanism under a controlled signal list.

The Architect approved publishing diagnostic commit `f1fdd58`. Native
execution is tracked in
[CI run 45058609883](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45058609883).
Those runs exercise the diagnostic revision and use its separate
`fnv1a64:4ad973daec75238a` fingerprint.

### Native macOS finding

The [first diagnostic macOS job](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45058609883/job/137319291123)
reproduced the stdout timeout in the plain Git suite. The group snapshot
contained the unreaped Git root, PID 18968 in state `Z`, and a live
`sleep`, PID 18974 in state `S`. Both had PGID 18968. The second signal
through the original owner succeeded, stdout reached EOF, and collection
reported root signal 9. The original timeout still failed the test and
prevented coverage emission. This proves that root exit did not establish
shutdown of the owned group in this case. The trace does not time the
descendant's fork relative to the kernel's member snapshot.

The downloaded macOS job log has SHA-256
`307e7ba3921d28014501252fae3ba8536bc34f83a4a173286b5c22e075cc1f68`.
The Windows and Linux jobs passed the plain and coverage-instrumented test
suites and reached the missing new-crate coverage floors. The TypeScript,
docs integrity and attribution jobs passed.

Apple's [Darwin group signal source](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_sig.c#L1576)
uses [a member snapshot](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_proc.c#L4058)
that it releases before delivering signals. This mechanism is consistent
with the observed survivor. The trace itself establishes the survivor's
membership and liveness after the first request.

### Local repair verification

The local repair keeps the macOS root unreaped while shutdown polling checks
the group. A successful group signal leaves confirmation pending. After root
exit, a missing group or an exact lone-root observation confirms shutdown.
Git uses its existing deadlines. A deterministic macOS regression sends the
first signal only to the fixture root, verifies that a live member in the
same group still holds stdout, and checks that the first poll issues another
signal while the root is still unreaped. A second regression covers detached
reaping under the same conditions. Native execution remains pending.

Darwin Clippy passed with the process, terminal and Git tests compiled for
`aarch64-apple-darwin`. A disposable Linux guest passed the eight process
tests and both focused Git shutdown tests. The first local Windows pass
completed 119 Git, 16 native process and 23 PTY tests.

The follow-up Windows gate, launched through `cmd.exe`, exposed another
launch case. The Agora failed to start Git because the native environment
encoder rejected a name containing `=`. A temporary Rust program enumerated
only unusual inherited names and observed `=C:` under `cmd.exe`.
[CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw#remarks)
documents these per-drive directory entries in custom environment blocks.
The encoder now accepts exactly that drive-name shape and retains validation
for ordinary entries. The ordinal encoding test checks the last alias and
its position in the block. The same Git assertion under `cmd.exe` went from
no exit code before the fix to Git's expected exit code after it.

All 93 `eph-agora` tests passed when rerun through `cmd.exe`. The focused
Windows `cargo-mutants 27.1.0` round caught all six environment encoder
mutations. Both viable control mutations missed with zero tests running.
Both baselines passed, there were no timeouts, and all 82 tracked Rust
inputs had unchanged hashes. The PowerShell gate passed its first six
steps and the instrumented suites, then reported only the two missing
coverage floors.

The full Windows `cargo xtask gate` rerun through `cmd.exe` reached the
same boundary. Both test builds passed, all existing coverage floors held,
and the only two findings were the unseeded `eph-process` and `eph-pty`
floors. These local measurements do not seed the floors.

The focused Linux guest round selected the shared `Child::terminate_group`,
`Child::signal_group`, and `Child::group_signal_failure` bodies. The
eight-test baseline passed. Each of the five viable mutations failed one
Rust test with exit code 101. The two viable control mutations both missed
with zero tests running. There were no unviable mutations or timeouts, and
all 82 tracked Rust inputs had unchanged hashes.

The Darwin workflow now selects the shared `Child::try_wait` guard that
permits root collection. Linux's terminal-specific readiness checks are
separate, so the Darwin selection has 31 candidates and omits Linux-only
branches. Bash accepted the workflow step. Its validator accepted the
recorded Linux round and control, and rejected copies with a failed
baseline, a survivor, a timeout, a control catch, or a wrong control exit
status. This exercises the workflow checks locally. Native Darwin
execution remains pending.

The Linux regression leaves a same-group descendant ignoring hangup after
the terminal root exits. The real wait path stops that descendant and the
terminal output reaches EOF. All nine process tests and both focused Git
shutdown tests passed in the guest. The reap-guard round caught all six
viable mutations through Rust test failures. One mutation did not compile
because `ExitStatus` has no `Default` implementation. Both controls missed
with zero tests running, there were no timeouts, and the hashes of all 82
tracked Rust inputs matched. Darwin cross-Clippy passed on the same guard
and fixture changes.

### Approved macOS confirmation interval

On 2026-11-23 the Architect approved reporting an unconfirmed PTY shutdown
after 500 ms while retaining ownership for later polls. The interval starts
at the first forced request or first observation of natural root exit.
The timeout test starts with an expired interval and a live output holder.
It requires a timeout, an unreaped root, EOF after another owned-group
signal, and the original root status from a later poll. A running terminal
with no shutdown request accrues no cleanup time. The detached PTY
regression also enters disposal with an expired foreground interval and
checks that the worker retries before collecting the root. A detached Git
worker uses the remaining time from the original request.

`cargo clippy --locked --target aarch64-apple-darwin -p eph-process -p eph-pty -p eph-git --tests -- -D warnings`
passed with the new regressions compiled. The expanded Darwin workflow
lists 33 mutation candidates. Native tests and the native mutation round
remain required.

The Windows gate through `cmd.exe` passed formatting, Clippy, the plain
and instrumented workspace suites, dependency review and invariant
checks on the approved repair. The five frozen-tree checks passed. The
gate reported only the two missing CI-seeded coverage floors.

### First native baseline on the repaired code

The [macOS mutation job on `6eb436b`](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45118329422/job/137506149894)
compiled the unmodified process tests. Ten passed, including the 500 ms
timeout and recovery test. The retry and detached cleanup tests failed
because the old fixture's output channel was already ready after it had
killed the root. Cargo-mutants stopped before testing any mutations. Its
no-test control missed all nine selected mutations. The
[regular macOS gate](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45118329351/job/137506149867)
passed the real Git adapter test, reproduced the two fixture failures
and produced no coverage receipt.

The retry regressions now use the Rust group fixture. The descendant
reports its own PID over the output stream. The test checks its PGID
before and after signaling only the root, requires the output to stay
open, then requires EOF after the production shutdown operation. It keeps
the piped stderr handle open while observing stdout. An early output
result now appears in the failure message.

The artifact recorded an unmodified baseline for `eph-process` alone
and selected both `eph-process` and `eph-pty` for individual control
mutations. The workflow now checks both packages on the unmodified tree
before running the round.

All nine Linux process tests passed in the guest with the Rust fixture,
and Darwin cross-Clippy accepted both test packages. The Windows gate
through `cmd.exe` passed both test builds and stopped at the two
unseeded floors. The updated workflow passed Bash's syntax check.

### Observe Darwin process liveness separately from terminal output

The [first native run of the Rust fixture](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45119534055/job/137509881721)
reproduced the two early output completions. The new diagnostic recorded
`Ok(Ok(()))`, so the reader had reached EOF rather than returning a read
error. The 500 ms timeout and recovery regression passed again.

The [XNU session-leader exit path](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/kern_exit.c#L2093)
can revoke its controlling terminal. The Darwin regression now observes
the actual descendant with [libproc's short BSD query](https://raw.githubusercontent.com/apple-oss-distributions/xnu/main/bsd/kern/proc_info.c#L2046).
It requires a live member of the owned group after the root exits and
verifies that the member stops after the production shutdown step.
It still requires a piped fixture to hold stdout open before shutdown.
On a PTY, terminal EOF and process liveness are distinct assertions.

Darwin cross-Clippy passed with the liveness probe, and all nine Linux
process tests passed in the guest. The Windows gate passed both test
builds and reached the two pending coverage floors. The frozen-tree
checks passed.

The [native proof on `68cfe77`](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45122195861/job/137518281696)
passed all 12 process and 22 PTY tests on Darwin 25.6.0 ARM64 with Rust
1.96.0. The mutation tool ran 33 candidates. All 30 viable changes
failed Rust tests. The compiler rejected three changes because
`Instant` cannot be multiplied by a `Duration`, let chains cannot use
`||`, and `ExitStatus` has no `Default` implementation. No mutations
survived or timed out. All nine control mutations missed with zero
tests running. The hashes of all 82 tracked Rust inputs matched.
The [proof artifact](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45122195861/artifacts/14794112535)
contains the baseline, round, control and hashes.

### Corroborated coverage floors

[CI run 45122195829](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45122195829)
passed the plain and coverage-instrumented workspace suites on Linux,
macOS and Windows in each of its first three attempts. The Rust gates
reported only the two unseeded floors. All nine receipts name
`68cfe77ae31c7e93871ea52f44399753c993d538`, tree
`fnv1a64:24b271b44e7dc27b` and `cargo-llvm-cov 0.9.1`. The artifact
names, platform and attempt identities matched, and every report listed
zero untested modules.

`cargo xtask coverage ratchet` generated the floor record from all nine
receipts. The new rows show the minimum measured coverage on each
platform.

| Crate | Floor | Linux | macOS | Windows |
| --- | ---: | ---: | ---: | ---: |
| `eph-process` | 94.88% | 94.88% | 95.15% | 95.70% |
| `eph-pty` | 94.94% | 95.04% | 95.04% | 94.94% |

The ratchet also raised `eph-agora` to 98.84%, `eph-git` to 99.55% and
`xtask` to 98.29%. No floor fell.

The final local `cargo xtask gate` passed through `cmd.exe` on Windows.
The coverage check reported success across seven crates and 45
production files. The Windows figures for `eph-process` and `eph-pty`
matched their CI receipts at 95.70% and 94.94%.

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

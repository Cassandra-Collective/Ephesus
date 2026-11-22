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

The public caller plant, dedicated encoding-return and attribute-pointer
proofs, remaining Linux classification, current-source gates and the
three-attempt native CI coverage ratchet remain pending. Earlier CI
measurements cannot seed the changed source's floors.

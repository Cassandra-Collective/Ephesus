# R2.3 — Agent lifecycle and terminal commands

**Status** verification in progress · **Branch** `feature/r2-3-eph-agents`

## Decisions

The Architect approved SDD-RUST §3.4, decisions 1A–5A, A1–C1, D1–E1, and F1
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
- **Tier A.** The Windows archive of `6d89ec18519ff3fa570b9af65993ae95176b370a`
  matched all 1,138 Git blobs before testing. The first plain run exited
  101 because the descendant readiness marker was missing at the probe
  fixture's one-second deadline. The second run exited 0. That proof
  series stopped after the Architect approved the fixture repair.
- **Tier C.** F1 makes the fixture wait up to five seconds for the real
  descendant's readiness marker before starting its one-second assertion.
  Startup failure is distinct from the deadline the test intends to
  observe. The production probe still captures its ten-second deadline
  before launching, and both paths use its owned cleanup.
- **Tier A.** Windows `cargo clippy --locked -p eph-engines --all-targets --
  -D warnings` and `cargo test --locked -p eph-engines` exited 0 with all
  27 tests passing. The repaired descendant test passed on Windows and
  WSL2 under `cargo llvm-cov --locked -p eph-engines --no-report --
  a_descendant_holding_the_pipes_keeps_the_probe_incomplete` and also
  passed in a plain focused WSL2 run.
- **Tier C.** A read-only check of `199668d` confirmed that both production
  probe entry points capture the deadline before launch and retain their
  cleanup path. The fixture checks a deadline result and elapsed time after
  descendant readiness. That observation includes cleanup, so it does not
  measure the exact instant the waiter returned. Root exit is not observed
  independently, and the five-second startup limit is checked between polls.
- **Tier A.** On the Windows archive of `199668d`, all 1,138 Git blobs
  matched before testing. Ten plain workspace runs and the first
  instrumented workspace run exited 0. The second instrumented run
  exited 101 at `a_slow_read_of_the_book_is_reported_as_log_size_with_what_it_read`,
  where the test expected the live read to finish below 50 ms. The log
  recorded `94 passed; 1 failed` for `eph-agora`.
- **Tier A.** The third instrumented run of the same archive exited 101
  in `an_isolated_respawn_keeps_dirty_work_and_rejects_stale_events`.
  The headless driver reached its 12-second deadline in `Checking`.
  The failure log showed `problems []` and journal events `requested`
  and `started`. It does not identify which pending operation was late.
- **Tier A.** The fourth instrumented run failed in
  `ordinary_launch_controls_and_unwinds_a_real_terminal`. Its fixture
  reported `Refused("the engine probe exceeded its deadline")` with a
  `requested`, `refused`, `raised`, `unwound` journal. This uses the real
  probe path with a compiled fixture executable. The log does not
  identify why the subprocess missed the deadline.
- **Tier A.** The Windows series stopped between runs and rechecked all
  1,138 Git blobs against `199668d`. The archived coverage-instrumented
  headless binary then ran three isolated replays with `RUST_BACKTRACE=1`.
  Two exited 101 and one exited 0. The full headless replay also exited
  101. The three backtraces identify the wait for the initial isolated
  launch at `headless.rs:640`. Each reported `Checking` with only
  `requested` in the journal and a Windows sharing violation during test
  cleanup. One retained fixture contained a raw coverage profile in its
  target directory. The file owner and the operation that consumed the
  launch wait remain unverified.
- **Tier A.** A later Windows replay on the same verified archive passed
  both plain and both instrumented single-case runs. A full instrumented
  run still exited 101 at `headless.rs:640`. The CPU load sample was 29%
  before the full replay. The samples cover the starts of the replays,
  and the results leave the cause of the initial launch delay open.
- **Tier A.** `llvm-profdata merge` and `llvm-cov show` on the four
  retained target-directory profiles each showed one hit in the fake
  engine's `--version` arm and zero in its `auth` arm. The profile writes
  were 1.064, 4.721, 7.032 and 9.965 seconds after the respective
  `requested` records. One fixture retained its Git worktree
  registration. These are timestamps for the profile writes. The process
  exit and subsequent stage timings remain unmeasured.
- **Tier A.** The first Windows `cargo xtask mutants eph-schema` used
  G2's automatic timeout on the verified `199668d` archive. The
  unmutated baseline passed (`97s build + 3s test`), and the automatic
  test limit was 20 seconds. During the round, the first `TIMEOUT`
  reported 88 of 90 test results, including 44 failures. The other
  seven timeout logs contained no individual test result.
  The report stopped at 207 of 336 candidates with 172 caught, 27
  unviable, zero misses and eight timeouts. The runner was gone and the
  control had no report when checked after Windows recorded an unexpected
  shutdown. The eight timed-out cases earn no mutation kills. The event
  log does not explain the earlier test timeouts.
- **Tier A.** The Windows archive's plain headless executable selected
  zero cases for `__ephesus_mutants_control_matches_no_test__` and listed
  nine cases without the filter. Its mutation control remains owed.
- **Tier A.** On WSL2 with Cargo 1.96.0 and Git 2.53.0, the verified
  archive of `199668d` passed all ten `cargo test --workspace --locked`
  runs. All 1,138 Git blobs and executable bits matched before and after
  the series. Earlier launcher attempts stopped before running tests.
  The instrumented WSL2 series and the final candidate's repeated proof
  are still owed.
- **Tier A.** WSL2 `cargo xtask mutants eph-pty` ran on that archive
  with the approved cargo-mutants 27.1.0 (`ephesus-mutants-eph-pty-3663`).
  It exited 1 with 100 caught, three missed, 19 unviable and zero
  timeouts from 122 candidates. The no-tests control missed 30 of
  30 viable mutants, with one unviable and zero timeouts. Both baselines
  passed and the harness compared 82 unchanged files.
- **Tier C.** Two PTY survivors change which inspection errors count
  as candidate absence. The third is inside the Windows-only PATHEXT
  parser and still needs native Windows evidence.
- **Tier C.** L1 shares the `NotFound` classification between the two
  lookup metadata reads. The final read that follows an existing
  ancestor still propagates a dangling link's error. The read-only
  review confirmed that the Windows fixture would choose the deliberately
  present extensionless file if filtering retained only blank entries.
  The error classification test supplies synthetic errors. Native ACL
  behavior is outside that test's evidence.
- **Tier A.** After L1, `cargo clippy --locked -p eph-pty --all-targets
  -- -D warnings` and `cargo test --locked -p eph-pty` exited 0 on Windows
  and WSL2. Windows passed 30 tests, including the native PATHEXT fixture.
  WSL2 passed 29, including the real dangling-component fixture. The
  updated PTY mutation round remains owed.
- **Tier A.** The first WSL2 `cargo xtask mutants eph-git` exited 3
  before measuring any round mutants. Seven unmutated tests failed.
  The launcher's `TMPDIR` was under the WSL home repository.
  `git rev-parse --show-toplevel` from there returned `/home/senso`,
  and the same command from `/tmp` reported no repository. The control
  missed all 60 viable cases, but the failed baseline voids the round.
- **Tier A.** From a fresh `/tmp` directory outside that repository, the
  WSL2 Git command passed both baselines (`ephesus-mutants-eph-git-594`).
  It exited 3 with 189 caught, six misses, 67 unviable and one timeout
  out of 263. The forced-false root-exit case reported failing tests
  before its 30-second limit. The control missed all 60 viable cases
  with six unviable and zero timeouts. The harness compared 82 unchanged
  source files.
- **Tier C.** The six Git misses are inside Windows launch, native
  shutdown confirmation or verbatim worktree path handling. Their
  Windows verdicts remain owed.
- **Tier A.** The M1 WSL2 Git run on `6cc032e` reached the approved
  240-second limit on the same root-exit mutant. Its log shows 117 of
  118 test results, including 95 failures. The only test without a
  result was `a_deadline_beyond_what_an_instant_can_hold_is_clamped`.
  The round again exited 3 with 189 caught, six misses, 67 unviable
  and one timeout. The control missed 60 of 60 viable with zero
  timeouts. All 1,138 blobs and executable bits matched before and after.
  Raw reports are retained under the WSL2 proof directory, out of `/tmp`.
- **Tier C.** At that commit, the clamp test launched Git with the
  one-hour maximum. N1 preserves the direct clamp assertion and uses
  two seconds for the real Git call. H1 uses the Agora fixture's private
  threshold override for both read outcomes and checks the default on
  the opened Agora. J1 gives only the initial isolated launch a 60-second
  fixture wait and reports a refusal immediately. Production deadlines
  are as specified before these fixture edits.
- **Tier A.** Windows targeted Clippy rejected an explicit panic in the
  standalone headless harness. The fixture now uses a test assertion
  and Clippy exited 0. The amended Windows suite passed five manager
  unit tests, all nine headless tests and 95 Agora tests. Its Git clamp
  test passed, but two existing Git pipe fixtures failed at their
  two-second root deadline. Both passed twice in focused replays. A
  second full Windows Git run had two deadline failures across those
  fixtures and another pipe fixture. The clamp test passed again.
- **Tier A.** On WSL2, `cargo clippy --locked -p eph-agora -p eph-agents
  -p eph-git --all-targets -- -D warnings` and `cargo test --locked
  -p eph-agora -p eph-agents -p eph-git` exited 0 after H1, J1 and N1.
  Five manager unit tests, nine headless cases, 97 Agora tests and 118
  Git tests passed. The Windows pipe fixtures and exact repeated proof
  were still owed at that point.
- **Tier A.** After P1, `cargo clippy --locked -p eph-git --all-targets
  -- -D warnings` and `cargo test --locked -p eph-git` exited 0 on Windows.
  All 122 Git tests passed, including the three pipe fixtures. The WSL2
  Clippy and test run for all three affected packages also exited 0 with
  five manager unit tests, nine headless cases, 97 Agora tests and 118
  Git tests. These are package runs before the repeated I1 proof.
- **Tier C.** P1 stages the actual root exit and holder marker within an
  eight-second fixture bound, then uses the existing drain grace. The
  late-marker case releases the holder after completion and observes
  whether it can still write. On Windows that observation combines
  explicit termination and job-handle close. It does not distinguish
  which stopped the holder, and the observation window remains bounded.
- **Tier A.** The O1 WSL2 `cargo xtask mutants eph-git` round on
  `ff07411` caught 190 of 263 candidates, with six misses, 67 unviable
  and zero timeouts. The previously timed-out root-exit case was caught
  after 108.28 seconds in the test phase. The no-tests control missed
  all 60 viable cases with six unviable and zero timeouts. The command
  exited 1 because of the six Windows-only misses. The mutation harness
  compared 82 unchanged files, and all 1,138 Git blobs and executable
  bits matched before and after the run. The stage retained both reports.
- **Tier A.** On the verified Windows archive of `ff07411`, the six Git
  cases missed on WSL2 produced five catches and one compile rejection
  because `Piped` has no `Default` implementation. The no-tests control
  missed all five viable cases. The targeted PTY replay produced six
  catches and one compile rejection because `Metadata` has no `Default`
  implementation. The PTY control missed all six viable cases. Neither
  replay nor its control timed out, and all 1,138 Git blobs still matched
  afterward.
- **Tier A.** Windows `cargo xtask mutants eph-schema` on `ff07411`
  exited 0 with `ROUND OK` under the approved K1 90-second launcher limit.
  It caught all 290 viable cases, including the eight previously timed-out
  cases. The 46 remaining cases each had a compiler diagnostic. The
  no-tests control missed all 75 viable cases, with nine unviable and
  zero timeouts. The harness compared 82 unchanged files, and all 1,138
  archive blobs matched before and after. Reports are under
  `ephesus-mutants-eph-schema-36816`.
- **Tier A.** The I1 Windows archive of `ff07411` passed all ten plain
  `cargo test --workspace --locked` runs and the independent LCOV run.
  All 1,138 archive blobs matched before and after. The first launcher
  supplied both `--no-report` and `--no-clean`. The tool rejected that
  combination before executing any of the ten instrumented tests.
  The retry ran `cargo llvm-cov --workspace --locked --no-report` on
  the same archive and exited 0 all ten times. Each log ran 13 test
  executables and recorded nine passing headless cases, with no
  recompilation. All 1,138 blobs matched before and after the retry.
  The separate LCOV report and original logs were retained.
- **Tier A.** The I1 WSL2 archive of `ff07411` passed all ten plain
  `cargo test --workspace --locked` runs, all ten `cargo llvm-cov
  --workspace --locked --no-report` runs, and the independent LCOV run.
  The first instrumented run built the suite. The other nine reused it
  with no recompilation. Each ran 13 test executables and recorded nine
  passing headless cases. All 1,138 blobs and executable bits matched
  before and after. The launcher exited 0.
- **Tier A.** The revised WSL2 `cargo xtask mutants eph-pty` round on
  `ff07411` caught 100 of 121 candidates with 20 unviable and zero
  timeouts. Its sole miss was `path.rs:70:33`, the Windows-only PATHEXT
  case caught by the native replay. The no-tests control missed all
  30 viable cases, with one unviable and zero timeouts. The command
  exited 1 because of the platform-only miss. The harness compared 82
  unchanged files, and all 1,138 archive blobs and executable bits
  matched before and after. Both reports were retained.
- **Tier A.** The WSL2 `cargo xtask mutants eph-engines` round on
  `ff07411` caught 171 of 257 cases, with 82 unviable and zero timeouts.
  The four misses were Windows-only reparse-point and job-shutdown
  cases. The no-tests control missed all 61 viable cases with four
  unviable and zero timeouts. The harness compared 82 unchanged files.
  The Windows targeted replay caught all four cases, and its control
  missed all four with zero timeouts. Both platforms verified all 1,138
  archive blobs before and after their runs, with WSL2 also checking
  executable bits. Both sets of reports were retained.
- **Tier A.** The WSL2 `cargo xtask mutants eph-agora` round on
  `ff07411` caught 223 of 306 cases with 81 unviable and zero timeouts.
  Its no-tests control missed all 69 viable cases, with eight unviable
  and zero timeouts. The two misses were the `File::open` absence guards
  in `Book::ends_mid_line` and `Book::snapshot`. The Windows selector
  included those two plus a `Book::since` cursor mutation. The first
  launcher stopped before testing because it expected two candidates.
  The retry caught all three. A later audit showed that it used the
  Git library left by the shared build directory's Git control. A
  rerun with a fresh Windows build directory for each phase also
  caught all three. Its control missed all three, with zero timeouts
  in either phase. Both platforms verified all 1,138 archive blobs
  before and after, with WSL2 also checking executable bits.
- **Tier A.** The WSL2 `cargo xtask mutants eph-agents` round on
  `ff07411` used the automatically selected 20-second test limit and
  exited 1 with survivors. It caught 166 of 311 cases, with 91 missed,
  54 unviable, and zero timeouts. The no-tests control missed all 71
  viable cases, with seven unviable and zero timeouts. Both baselines
  passed. The harness compared 82 unchanged files, and all 1,138
  archive blobs and executable bits matched before and after. Both
  raw reports were retained. Compiler logs accounted for all rejected
  builds. The round had 39 trait errors, six invalid binary operators,
  and nine unsupported `||` let-chain expressions. All seven control
  rejections were trait errors.
- **Tier A.** The first Windows `eph-agents` replay stopped at the
  unmutated baseline. It reused the Git library left by a control
  mutation that disables drive-prefix conversion. Git refused the
  resulting verbatim worktree path. With a fresh Windows build
  directory for each phase, the replay caught all three reparse-bit
  mutations, and the control missed all three with zero timeouts
  in either phase. All 1,138 archive blobs matched before and after.
- **Tier C.** Review traced the agent survivors to launch validation,
  journal writes, terminal control, and respawn. Three reparse-bit
  mutations run only on Windows. The native replay resolves those
  three. Follow-up verification remains for the other 88 survivors.
- **Tier C.** Refutation identified a late capacity release after
  shutdown that can distinguish both respawn guard mutations. The
  reservation expression and unconditional key acknowledgement each
  have a candidate equivalent mutation under the existing owner
  transitions. These are source findings awaiting follow-up proof.
- **Tier A.** A standalone Rust probe returned `InvalidInput` for an
  embedded NUL in `symlink_metadata` on both Windows and WSL2. A missing
  component returned `NotFound` on both. Using a regular file as a
  parent returned `NotFound` on Windows and `NotADirectory` on WSL2.
  Creating a directory below an absent parent returned `NotFound` on
  both. These are observations of the standard library calls. The
  manager's propagation of these results still needs a regression test.
- **Tier A.** The Windows `cargo xtask gate` preflight on `ff07411`
  passed formatting, Clippy, workspace tests, dependency deny checks,
  invariants, and the approved-dependency check. Coverage measured
  `eph-agents` at 81.17% (1,466 of 1,806 lines). Every existing floor
  passed, and the gate exited 1 solely for the absent new-crate floor.
  All 1,138 archive blobs matched the commit before and after.
- **Tier A.** The Q1 and R1 follow-up passed 18 unit tests, 14
  headless tests, and package Clippy on native Windows in the working
  tree. The first unit-only run found that the journal failure fixture
  tried to move the book before any line created it. The fixture now
  acknowledges a record and drains the committer before damaging that
  book. The passing suite includes the repaired fixture.
- **Tier C.** The follow-up exercises real installer input, stale
  generations, authentication and tool results, effective working
  directories, durable degradation, and the two respawn guards. Private
  hooks supply controlled directory creation and terminal observations
  alongside the real filesystem and PTY fixtures. The working-tree run
  does not establish the remaining mutation verdicts or final-tree proof.
- **Tier C.** Read-only review found a Windows fixture that would claim
  a process exemption and test-only module declarations the coverage
  classifier would reject. The fixture now uses the existing PTY batch
  launcher, and the shared test files are included inside inline test
  modules. A late capacity release after shutdown can still leave an
  expired respawn deadline. The regression establishes that it launches
  no hire. This observation led to the separate S1 decision below.
- **Tier A.** After those fixture repairs, Windows again passed all 18
  unit tests, all 14 headless tests, and package Clippy. The invariant
  and approved-dependency checks each passed over nine crates and 75
  source files. Direct `cmd.exe` arguments under the PTY failed in the
  diagnostic runs. The Windows batch fixture passed through the approved
  launcher and exercised the real junction.
- **Tier A.** WSL2 `cargo xtask mutants eph-agents` on `eb47593`
  caught 255 of 314 cases, with 54 unviable, five missed and zero
  timeouts. The control missed all 72 viable cases, with seven unviable
  and zero timeouts. Both baselines passed. The harness compared 84
  unchanged files. The Windows replay caught the three reparse-bit
  survivors on a fresh build target and missed all three in its
  separate no-tests control, with zero timeouts in either phase. Both
  platforms verified all 1,140 archive files before and after, with
  WSL2 also checking executable bits. The WSL2 compiler logs account
  for all rejected builds. The round had 39 trait errors, six invalid
  binary operators and nine unsupported `||` let-chain expressions.
  The seven control rejections were trait errors. Attribution on
  `eb47593` passed.
- **Tier C.** The two remaining survivors change the reservation
  predicate's token and checkout checks and the key-acknowledgement
  guard. The read-only review found both equivalent on reachable
  manager transitions. T1 records the Architect's disposition below.
- **Tier A.** The Windows gate on `eb47593` passed formatting, Clippy,
  tests, deny checks, invariants, and approved dependencies. The
  verifier rejected the two shared-fixture paths with `src/../tests`
  in their coverage records. The fixture imports now use absolute
  crate paths. After that correction, `cargo xtask coverage` in the
  Windows working tree passed all instrumented tests and accepted the
  source paths. It measured `eph-agents` at 89.16% (1,646 of 1,846
  lines) and exited 1 solely for the absent new-crate floor. Every
  existing floor passed. The checked-in floor file was untouched.
- **Tier A.** An exact WSL2 archive of `cf0b259` passed package
  Clippy and the instrumented workspace suite. Its LCOV report omitted
  the test fixtures. The coverage verifier measured `eph-agents` at
  89.21% (1,647 of 1,846 lines), passed every existing floor, and
  exited 1 solely for the absent agent floor. All 1,140 Git blobs and
  executable bits matched the commit before and after.
- **Tier A.** The exact Windows gate on `cf0b259` passed formatting,
  Clippy, plain workspace tests, deny checks, invariants, approved
  dependencies, and the instrumented workspace tests. Coverage
  recognized the fixture records under `tests/support` and excluded
  them from the production tally. It measured `eph-agents` at 89.16%
  (1,646 of 1,846 lines). Every existing floor passed, and the gate
  exited 1 solely for the absent agent floor. All 1,140 archive blobs
  matched the commit before and after. Attribution passed at `cf0b259`.
- **Tier C.** The approved S1 release records a deferred exit without
  arming a deadline once shutdown begins. T1 relies on the `Cleaning`
  state for retained ownership and matches acknowledged keys by input
  kind. The U1 follow-up removes duplicate checks at due dispatch.
  Read-only review and refutation traced deadline creation to settled,
  uncancelled slots while running. Manual replacement clears the old
  deadline, automatic replacement consumes it, and cancellation and
  shutdown clear it. The owner still checks generation and invokes
  admission before replacement and again before PTY spawn.
- **Tier A.** Native Windows passed all 18 unit tests, all 14 headless
  tests, package Clippy and the invariant check after S1, T1 and U1.
  The strengthened headless case checks for an immediate wake before
  the first drive, then verifies no remaining deadline and a single
  `released` record correlated with the same hire, request and rung.
- **Tier A.** The WSL2 `cargo xtask mutants eph-agents` round on
  `1dbbdfc` caught 251 of 308 cases with 54 unviable, three misses,
  and zero timeouts. Its no-tests control missed all 70 viable cases,
  with seven unviable and zero timeouts. The compiler logs account
  for every rejected build. Windows caught the three reparse-only
  misses and its no-tests control missed all three, with zero timeouts
  in either phase. Windows used a fresh target for each phase. All
  1,140 archive blobs matched before and after on both platforms,
  with executable bits also checked on WSL2. Attribution passed on
  `1dbbdfc`.
- **Tier C.** The pre-close audit found only component or adjacent
  evidence for the manager's post-sync journal failure, overlapping
  roster and ledger edits, and a checkout acquisition returned after
  cancellation. SDD-RUST §3.4 requires these cases. The Architect
  approved the follow-up test approach in V1 and W1.
- **Tier A.** Both existing focused Book tests passed on verified
  Windows and WSL2 archives of `1dbbdfc`. The fault-return test found an on-disk
  line only after the write and no subscriber notification at either
  fault point. The abrupt-exit test recovered the synced line without
  an acknowledgement and verified that the book could append again.
  All 1,140 archive blobs matched before and after on both platforms,
  with executable bits also checked on WSL2. This verifies the Book
  boundary. The manager response is still owed.

## Recovery before laptop reset

The Architect paused implementation on 2026-11-26 and requested publication
of the current work in a draft for recovery. The R2.3 draft is stacked on
the published R2.2 branch. Use this record to resume the implementation.

- **Tier B.** Published R2.2 closeout `9660bba286441fdfd814403d493cf435230705aa`
  passed Linux, macOS, and Windows CI in [run 45552342374](https://github.com/Cassandra-Collective/Ephesus/actions/runs/45552342374).
  Its predecessor trial merge against `main` at `9784131` was clean.
- **Tier A.** The last tested R2.3 checkpoint is
  `1dbbdfce2f65e39afd7621c0cd92ecffcfd9f538`. Above are its Windows
  package results and the exact WSL2 mutation round and Windows replay.
  Their controls missed every viable case and there were zero timeouts.
- **Tier C.** The recovery commit includes the V1 Agora test feature,
  the opt-in constructor for the real Book fault, the pre-write and
  after-sync manager fixtures, the real checkout cancellation fixture,
  and the overlapping roster and ledger cases. It also includes the
  shared native-link fixture in `tests/support/scratch.rs`.
- **Tier A and C.** The initial Windows V1 test stopped at
  `fixture_boot_attaches_the_fault_to_the_shared_book`. The test draft
  was a degradation, which the Agora intentionally refuses at that
  API. The saved edit uses a typed lifecycle record. The edit and the
  new headless cases await their first passing run.

Resume by running the corrected Agora fixture, then the full headless
suite. Complete the composed relative `PATH`, competing `PATH` grant,
redirect and overlap checks in SDD-RUST §3.4. Recheck the mutation inputs
for `eph-agora` because V1 changes its source and feature setup. Verify
the affected crates and repeat the complete plain and instrumented
workspace suites on Windows and WSL2 against the final commit. Seed the
`eph-agents` floor using three CI attempts on one tree. Close R2.3 only
after the required gates and three-platform CI pass. R2 exit still owes
the signed-in reference terminal, effective grants, conformance table,
Gymnasium metric, and carried boot conditions.

## Owed evidence

The agent mutation verdict and repeated Windows and WSL2 proofs are
owed on the final source tree. The seeded coverage floor, final gates,
approved publication, and three-platform CI are still owed. The fixture
uses real PTYs and local test executables. The signed-in reference-engine
PTY run and effective grant checks are owed at R2 exit.

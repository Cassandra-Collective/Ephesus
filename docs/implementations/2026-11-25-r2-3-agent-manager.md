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

## Owed evidence

Mutation verdicts for every changed crate, exact-tree repeated Windows and
WSL2 tests, the seeded coverage floor and final gates, approved publication,
and three-platform CI are still owed. The fixture uses real PTYs and local
test executables. The signed-in reference-engine PTY run and effective grant
checks are owed at R2 exit.

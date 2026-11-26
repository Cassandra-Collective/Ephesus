//! The manager runs both the installer and reference-shaped fixture in real PTYs.
//! A renamed copy of this binary supplies the external engine fixture. The test
//! entry point honors Cargo's positional filter, including the no-tests control.

#![expect(
    clippy::unwrap_used,
    reason = "headless fixture failures fail the custom test executable"
)]

use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[path = "support/fixture_hire.rs"]
mod fixture_hire;
#[path = "support/scratch.rs"]
mod scratch;
use scratch::Scratch;

use eph_agents::{
    CommandPhase, DeclaredTool, Error, Event, ExitPolicy, Generation, Hire, HookRegistration,
    Lifecycle, Manager, Services, TerminalSize,
};
use eph_agora::{
    agora::Agora,
    book::AppendPoint,
    store::{Loaded, OnDamaged, Store},
};
use eph_engines::{AdapterRegistry, Claude, ConfigDir, ToolRoot};
use eph_fs::atomic::write_atomic;
use eph_git::{committer::Hooks, runner::Git};
use eph_schema::{
    degradation::{Cause, Source as DegradationSource},
    id::TaskId,
    lifecycle::TerminalKind,
    log::{LineRead, read_line},
    names::{AgentId, EnvName},
    refs::Ref,
    restart::{BreakerStops, Signal, Stop},
    tasks::{Artifacts, Source, Task, TaskStatus, Tasks},
    time::Timestamp,
};

const TOKEN: &str = "fixture-session-secret";
const GRANT: &str = "fixture-granted-secret";

fn main() -> ExitCode {
    let exe = env::current_exe().unwrap();
    match exe.file_stem().and_then(|value| value.to_str()) {
        Some("claude") => return engine(),
        Some("npm") => return install(&exe),
        _ => {}
    }
    let args: Vec<_> = env::args().skip(1).collect();
    let filter = args.iter().find(|arg| !arg.starts_with('-'));
    let tests: &[(&str, fn())] = &[
        (
            "ordinary_launch_controls_and_unwinds_a_real_terminal",
            ordinary_launch,
        ),
        (
            "offer_is_explicit_and_install_then_auth_uses_the_same_terminal_identity",
            install_then_launch,
        ),
        (
            "nonzero_installer_exit_and_damaged_stop_cannot_continue",
            install_failure,
        ),
        (
            "a_failed_started_record_kills_the_terminal_and_unwinds",
            failed_started_record,
        ),
        (
            "a_requested_record_refused_before_write_stops_the_launch",
            failed_requested_record,
        ),
        (
            "a_synced_started_record_without_acknowledgement_unwinds_the_terminal",
            started_record_failed_after_sync,
        ),
        (
            "an_isolated_respawn_keeps_dirty_work_and_rejects_stale_events",
            isolated_respawn,
        ),
        (
            "a_checkout_returned_after_cancellation_is_unregistered",
            checkout_returned_after_cancellation,
        ),
        (
            "overlapping_hires_preserve_roster_edits_and_return_the_latest_ledger_tasks",
            overlapping_registry_and_ledger_edits,
        ),
        (
            "a_standing_stop_written_during_preparation_prevents_admission",
            stop_during_preparation,
        ),
        (
            "a_failed_revoke_reserves_only_its_hire_until_an_exact_retry",
            retry_reserved_cleanup,
        ),
        (
            "capacity_release_is_uncharged_and_durable_stop_vetoes_a_due_respawn",
            capacity_and_stop,
        ),
        (
            "an_installer_failure_cannot_spend_another_automatic_rung",
            install_failure_on_respawn,
        ),
        (
            "authentication_and_missing_tools_are_visible_before_launch",
            auth_and_tools,
        ),
        (
            "outstanding_registrations_with_an_empty_field_are_revoked",
            malformed_registration,
        ),
        (
            "only_the_matching_valid_standing_stop_blocks_a_hire",
            admission_and_stops,
        ),
        (
            "late_capacity_release_is_recorded_without_a_respawn_deadline",
            release_after_shutdown,
        ),
        (
            "a_confirmed_missing_transcript_disables_only_that_resume",
            resume_observed_session,
        ),
    ];
    let mut ran = 0;
    for &(name, test) in tests {
        if filter.is_some_and(|filter| !name.contains(filter)) {
            continue;
        }
        if args.iter().any(|arg| arg == "--list") {
            println!("{name}: test");
        } else {
            print!("test {name} ... ");
            test();
            ran += 1;
            println!("ok");
        }
    }
    if !args.iter().any(|arg| arg == "--list") {
        println!("{ran} headless test(s) passed");
    }
    ExitCode::SUCCESS
}

fn engine() -> ExitCode {
    match env::args().nth(1).as_deref() {
        Some("--version") => {
            let executable = env::current_exe().unwrap();
            let trace = executable.with_file_name("trace-version");
            if trace.is_file() {
                let cwd = fs::canonicalize(env::current_dir().unwrap()).unwrap();
                let mut paths = fs::read_to_string(&trace).unwrap();
                paths.push_str(cwd.to_str().unwrap());
                paths.push('\n');
                write_atomic(&trace, paths.as_bytes()).unwrap();
            }
            await_checkout_release(&executable);
            println!("2.1.295 (Claude Code)");
        }
        Some("auth") => {
            let logged_in = env::var_os("CLAUDE_CONFIG_DIR")
                .is_some_and(|dir| !Path::new(&dir).join("fixture-logout").exists())
                && env::var_os("EPH_HOOK_TOKEN").is_none()
                && env::var_os("FIXTURE_GRANT").is_none();
            println!(r#"{{"loggedIn":{logged_in}}}"#);
        }
        _ => {
            println!("manager-fixture-ready");
            println!("token={}", env::var("EPH_HOOK_TOKEN").unwrap_or_default());
            println!("grant={}", env::var("FIXTURE_GRANT").unwrap_or_default());
            let mut args = env::args().skip(1);
            if args.any(|arg| arg == "--resume") {
                println!("fixture-resume={}", args.next().unwrap());
            }
            io::stdout().flush().unwrap();
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line).unwrap();
            if line.trim().ends_with("fixture command") {
                println!("manager-fixture-accepted");
            }
        }
    }
    ExitCode::SUCCESS
}

fn await_checkout_release(executable: &Path) {
    if !executable.with_file_name("hold-checkout-probe").is_file() {
        return;
    }
    let cwd = fs::canonicalize(env::current_dir().unwrap()).unwrap();
    if !cwd.ends_with(Path::new("worktrees").join("agent.fixture")) {
        return;
    }
    let ready = executable.with_file_name("checkout-probe-ready");
    let release = executable.with_file_name("release-checkout-probe");
    write_atomic(&ready, cwd.to_str().unwrap().as_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !release.is_file() {
        assert!(Instant::now() < deadline, "checkout probe barrier expired");
        thread::sleep(Duration::from_millis(5));
    }
    write_atomic(
        &executable.with_file_name("checkout-probe-released"),
        b"released",
    )
    .unwrap();
}

fn install(exe: &Path) -> ExitCode {
    let cwd = env::current_dir().unwrap();
    assert_eq!(cwd.file_name().unwrap(), "engines");
    assert!(env::var_os("EPH_HOOK_TOKEN").is_none());
    assert!(env::var_os("FIXTURE_GRANT").is_none());
    assert!(env::var_os("CLAUDE_CONFIG_DIR").is_none());
    if cwd.join("interactive-install").exists() {
        println!("manager-fixture-installer-waiting");
        io::stdout().flush().unwrap();
        let mut confirmation = String::new();
        io::stdin().lock().read_line(&mut confirmation).unwrap();
        assert_eq!(confirmation.trim(), "confirm install");
        println!("manager-fixture-installer-confirmed");
    }
    let name = program("claude");
    copy_executable(exe, &exe.with_file_name(name));
    println!("manager-fixture-installed");
    if cwd.join("fail-install").exists() {
        return ExitCode::from(7);
    }
    ExitCode::SUCCESS
}

fn program(name: &str) -> String {
    format!("{name}{}", env::consts::EXE_SUFFIX)
}

fn copy_executable(from: &Path, to: &Path) {
    write_atomic(to, &fs::read(from).unwrap()).unwrap();
    fs::set_permissions(to, fs::metadata(from).unwrap().permissions()).unwrap();
}

#[derive(Default)]
struct Calls {
    registered: Vec<Generation>,
    revoked: Vec<Generation>,
    ended: Vec<Generation>,
    grants: Vec<String>,
    fail_next_revoke: bool,
    registration: Option<HookRegistration>,
    transcript_present: Option<bool>,
    transcripts_checked: Vec<String>,
}

struct Ports {
    calls: Arc<Mutex<Calls>>,
    fail_book: Option<PathBuf>,
    stop_file: Option<PathBuf>,
}

impl Services for Ports {
    fn grant(&mut self, _agent: &AgentId, name: &EnvName) -> Result<Option<String>, Error> {
        assert_eq!(name.as_str(), "FIXTURE_GRANT");
        self.calls.lock().unwrap().grants.push(name.to_string());
        Ok(Some(GRANT.into()))
    }

    fn memory(&mut self, _agent: &AgentId) -> Result<String, Error> {
        Ok("A fixture memory.".into())
    }

    fn register(
        &mut self,
        _agent: &AgentId,
        generation: Generation,
    ) -> Result<HookRegistration, Error> {
        self.calls.lock().unwrap().registered.push(generation);
        if let Some(file) = self.fail_book.take() {
            fs::rename(&file, file.with_extension("saved"))?;
            fs::create_dir(file)?;
        }
        if let Some(file) = self.stop_file.take() {
            write_atomic(&file, b"a damaged stop record")?;
        }
        Ok(self
            .calls
            .lock()
            .unwrap()
            .registration
            .take()
            .unwrap_or_else(|| HookRegistration {
                token: TOKEN.into(),
                endpoint: "fixture-endpoint".into(),
            }))
    }

    fn revoke(&mut self, _agent: &AgentId, generation: Generation) -> Result<(), Error> {
        let mut calls = self.calls.lock().unwrap();
        if std::mem::take(&mut calls.fail_next_revoke) {
            return Err(Error::Refused(format!(
                "fixture revoke could not complete: {TOKEN}"
            )));
        }
        calls.revoked.push(generation);
        Ok(())
    }

    fn ended(&mut self, _agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.calls.lock().unwrap().ended.push(generation);
        Ok(())
    }

    fn transcript_present(&mut self, _agent: &AgentId, session: &str) -> Option<bool> {
        let mut calls = self.calls.lock().unwrap();
        calls.transcripts_checked.push(session.into());
        calls.transcript_present
    }
}

struct Rig {
    manager: Manager<Ports>,
    agora: Arc<Agora>,
    calls: Arc<Mutex<Calls>>,
    home: PathBuf,
    target: PathBuf,
    bin: PathBuf,
    id: AgentId,
    output: Vec<(TerminalKind, Vec<u8>)>,
    problems: Vec<String>,
    commit_subjects: Arc<Mutex<Vec<String>>>,
    _scratch: Scratch,
}

impl Rig {
    fn new(installed: bool, fail_book: bool) -> Self {
        Self::with_agora(installed, fail_book, |root, prompts, hooks| {
            Agora::open(root, prompts, hooks).unwrap().agora
        })
    }

    fn with_agora(
        installed: bool,
        fail_book: bool,
        open: impl FnOnce(&Path, &Path, Hooks) -> Agora,
    ) -> Self {
        let scratch = Scratch::new();
        let home = scratch.0.join("home");
        let target = scratch.0.join("target");
        let bin = scratch.0.join("bin");
        for directory in [
            &home,
            &home.join("engines"),
            &home.join("prompts"),
            &target,
            &bin,
        ] {
            fs::create_dir(directory).unwrap();
        }
        if installed {
            copy_executable(&env::current_exe().unwrap(), &bin.join(program("claude")));
        } else {
            copy_executable(&env::current_exe().unwrap(), &bin.join(program("npm")));
        }
        let commit_subjects = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&commit_subjects);
        let hooks = Hooks::none().on_landing(move |landing| {
            observed
                .lock()
                .unwrap()
                .extend(landing.outcome.subjects.iter().cloned());
        });
        let agora = Arc::new(open(&home.join("agora"), &home.join("prompts"), hooks));
        let calls = Arc::new(Mutex::new(Calls::default()));
        let mut parent: Vec<(OsString, OsString)> = env::vars_os()
            .filter(|(name, _)| !name.as_encoded_bytes().eq_ignore_ascii_case(b"PATH"))
            .collect();
        parent.push(("PATH".into(), bin.as_os_str().to_owned()));
        let manager = Manager::new(
            &home,
            Arc::clone(&agora),
            AdapterRegistry::production(Claude::new(bin.join(program("eph-hook")))),
            Git::target().without_machine_config(),
            parent,
            TerminalSize {
                columns: 90,
                rows: 30,
            },
            Ports {
                calls: Arc::clone(&calls),
                fail_book: fail_book.then(|| agora.root().join("log.jsonl")),
                stop_file: None,
            },
        )
        .unwrap();
        Self {
            manager,
            agora,
            calls,
            home,
            target,
            bin,
            id: "agent.fixture".parse().unwrap(),
            output: Vec::new(),
            problems: Vec::new(),
            commit_subjects,
            _scratch: scratch,
        }
    }

    fn hire(&self) -> Hire {
        fixture_hire::hire(&self.id, &self.target)
    }

    fn start(&mut self) -> Generation {
        self.manager.start(self.hire()).unwrap()
    }

    fn drive_until(&mut self, ready: impl Fn(&Rig) -> bool) {
        self.drive_until_with_timeout(Duration::from_secs(12), ready);
    }

    fn drive_until_with_timeout(&mut self, timeout: Duration, ready: impl Fn(&Rig) -> bool) {
        let end = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            for event in self.manager.drive(now) {
                match event {
                    Event::Output {
                        terminal, bytes, ..
                    } => self.output.push((terminal, bytes)),
                    Event::Problem { detail, .. } => self.problems.push(detail),
                }
            }
            if ready(self) {
                break;
            }
            assert!(
                now < end,
                "lifecycle {:?}, problems {:?}, journal {:?}",
                self.state(),
                self.problems,
                self.agora.tail(24).map(|read| read
                    .entries
                    .into_iter()
                    .map(|entry| entry.event)
                    .collect::<Vec<_>>())
            );
            let scheduled = self.manager.next_deadline(now).unwrap_or(end);
            thread::sleep(
                scheduled
                    .saturating_duration_since(now)
                    .min(Duration::from_millis(5)),
            );
        }
    }

    fn state(&self) -> &Lifecycle {
        self.manager.state(&self.id).unwrap().1
    }

    fn text(&self, terminal: TerminalKind) -> String {
        let bytes: Vec<_> = self
            .output
            .iter()
            .filter(|(origin, _)| *origin == terminal)
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn degradation(&self, slug: &str) -> Option<String> {
        let cause = Cause {
            source: DegradationSource::Agents,
            slug: slug.into(),
            subject: Some(self.id.to_string()),
        };
        self.agora
            .degradations()
            .list()
            .into_iter()
            .find(|entry| entry.cause == cause)
            .map(|entry| entry.detail)
    }

    fn write_stop(&self, id: AgentId) {
        let mut stops = BreakerStops::empty();
        let _previous = stops.stops.insert(
            id,
            Stop {
                at: Timestamp::now(),
                signals: [Signal::Repetition].into(),
                detail: Vec::new(),
            },
        );
        Store::at(&self.home, OnDamaged::Block)
            .save(&stops)
            .unwrap();
    }

    fn target_repo(&self) {
        let git = Git::target().without_machine_config();
        for arguments in [
            &["init", "-b", "main"][..],
            &[
                "-c",
                "user.name=MERT EFE ŞENSOY",
                "-c",
                "user.email=sensoymertefe@gmail.com",
                "commit",
                "--allow-empty",
                "-m",
                "test: seed worktree fixture",
            ][..],
        ] {
            let result = git.run(&self.target, arguments);
            assert!(result.ok(), "{result:?}");
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.manager.shutdown();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.manager.drained() && Instant::now() < deadline {
            let _events = self.manager.drive(Instant::now());
            thread::sleep(Duration::from_millis(5));
        }
        self.agora.committer().drained();
    }
}

fn fixture_task(now: Timestamp, suffix: &str, status: TaskStatus, assignee: AgentId) -> Task {
    Task {
        id: TaskId::mint(now, suffix).unwrap(),
        title: "Fixture task".into(),
        spec: String::new(),
        assignee: Some(assignee),
        status,
        priority: 1,
        deps: Vec::new(),
        review: Default::default(),
        gates: Vec::new(),
        artifacts: Artifacts::default(),
        source: Source {
            kind: "fixture".into(),
            reference: Ref::Log(1),
        },
        created_at: now,
        updated_at: now,
    }
}

fn ordinary_launch() {
    let mut rig = Rig::new(true, false);
    let now = Timestamp::now();
    let mut ledger = Tasks::empty();
    ledger.tasks = vec![
        fixture_task(now, "11111111", TaskStatus::InProgress, rig.id.clone()),
        fixture_task(now, "22222222", TaskStatus::Review, rig.id.clone()),
        fixture_task(
            now,
            "33333333",
            TaskStatus::InProgress,
            "agent.other".parse().unwrap(),
        ),
    ];
    let _saved = rig.agora.write_tasks(&ledger).unwrap();
    let generation = rig.start();
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    rig.drive_until(|rig| {
        rig.state() == &Lifecycle::Running
            && rig
                .text(TerminalKind::Engine)
                .contains("manager-fixture-ready")
    });
    rig.manager
        .submit(&rig.id, generation, "discard".into())
        .unwrap();
    assert_eq!(rig.manager.held(&rig.id).unwrap().text, "discard");
    rig.manager.interrupt(&rig.id, generation).unwrap();
    assert!(rig.manager.held(&rig.id).is_none());
    rig.manager
        .observe_phase(&rig.id, generation, CommandPhase::Idle)
        .unwrap();
    rig.manager
        .submit(&rig.id, generation, "fixture command".into())
        .unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    let output = rig.text(TerminalKind::Engine);
    assert!(output.contains("manager-fixture-accepted"), "{output:?}");
    assert!(!output.contains(TOKEN));
    assert!(!output.contains(GRANT));
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.registered, [generation]);
    assert_eq!(calls.revoked, [generation]);
    assert_eq!(calls.ended, [generation]);
    let book = rig.agora.tail(20).unwrap();
    assert!(book.entries.iter().any(|line| line.event == "started"));
    assert!(book.entries.iter().any(|line| line.event == "unwound"));
    assert!(
        !book
            .entries
            .iter()
            .any(|line| line.event == "install-offered")
    );
    assert!(
        book.entries
            .iter()
            .any(|line| line.event == "offered" && line.data["memory"].as_bool() == Some(true))
    );
    let returned = book
        .entries
        .iter()
        .find(|line| line.event == "returned")
        .unwrap();
    assert_eq!(returned.data["reason"].as_str(), Some("agent-exit"));
    assert_eq!(returned.data["tasks"].as_array().unwrap().len(), 1);
    let ledger = match rig.agora.tasks() {
        Loaded::Present(ledger) => Some(ledger),
        _ => None,
    }
    .unwrap();
    assert_eq!(ledger.tasks[0].status, TaskStatus::Todo);
    assert_eq!(ledger.tasks[0].assignee.as_ref(), Some(&rig.id));
    assert_eq!(ledger.tasks[1].status, TaskStatus::Review);
    assert_eq!(ledger.tasks[2].status, TaskStatus::InProgress);
    rig.agora.committer().drained();
    assert!(
        rig.commit_subjects
            .lock()
            .unwrap()
            .iter()
            .any(|subject| subject == "agents: return unfinished work")
    );
    let history = Git::target()
        .without_machine_config()
        .run(rig.agora.root(), &["show", "HEAD:log.jsonl"]);
    assert!(history.ok(), "{history:?}");
    assert!(history.stdout.contains("\"event\":\"returned\""));
    assert!(!history.stdout.contains(GRANT));
}

fn install_then_launch() {
    let mut rig = Rig::new(false, false);
    let stale = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::InstallOffered(_)));
    assert!(!rig.bin.join(program("claude")).exists());
    assert!(rig.calls.lock().unwrap().grants.is_empty());
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    rig.manager.cancel(&rig.id, stale).unwrap();
    assert!(matches!(
        rig.manager.retry_cleanup(&rig.id, stale),
        Err(Error::Busy(_))
    ));
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped);
    assert!(!rig.home.join("worktrees").exists());
    write_atomic(&rig.home.join("engines/interactive-install"), b"yes").unwrap();
    let current = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::InstallOffered(_)));
    assert!(matches!(
        rig.manager.accept_install(&rig.id, stale),
        Err(Error::Busy(_))
    ));
    rig.manager.accept_install(&rig.id, current).unwrap();
    assert!(rig.manager.accept_install(&rig.id, current).is_err());
    rig.drive_until(|rig| {
        rig.state() == &Lifecycle::Installing
            && rig
                .text(TerminalKind::Installer)
                .contains("manager-fixture-installer-waiting")
    });
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    assert!(matches!(
        rig.manager.write_installer(&rig.id, stale, b"unexpected\r"),
        Err(Error::Busy(_))
    ));
    assert!(matches!(
        rig.manager.resize(
            &rig.id,
            stale,
            TerminalKind::Installer,
            TerminalSize {
                columns: 80,
                rows: 26
            }
        ),
        Err(Error::Busy(_))
    ));
    rig.manager
        .write_installer(&rig.id, current, b"confirm install\r")
        .unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    assert!(
        rig.text(TerminalKind::Installer)
            .contains("manager-fixture-installed")
    );
    assert!(
        rig.text(TerminalKind::Installer)
            .contains("manager-fixture-installer-confirmed")
    );
    assert!(matches!(
        rig.manager
            .write_installer(&rig.id, current, b"wrong terminal\r"),
        Err(Error::Busy(_))
    ));
    assert!(matches!(
        rig.manager.resize(
            &rig.id,
            current,
            TerminalKind::Installer,
            TerminalSize {
                columns: 80,
                rows: 26
            }
        ),
        Err(Error::Busy(_))
    ));
    rig.manager
        .resize(
            &rig.id,
            current,
            TerminalKind::Engine,
            TerminalSize {
                columns: 80,
                rows: 26,
            },
        )
        .unwrap();
    rig.manager.cancel(&rig.id, current).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped && rig.manager.drained());
    let book = rig.agora.tail(30).unwrap();
    assert_eq!(
        book.entries
            .iter()
            .filter(|entry| entry.event == "install-offered")
            .count(),
        2
    );
    assert!(!book.entries.iter().any(|entry| entry.event == "offered"));
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.registered, [current]);
    assert_eq!(calls.revoked, [current]);
}

fn install_failure() {
    let mut rig = Rig::new(false, false);
    write_atomic(&rig.home.join("engines/fail-install"), b"yes").unwrap();
    let generation = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::InstallOffered(_)));
    rig.manager.accept_install(&rig.id, generation).unwrap();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::Refused(_)));
    assert!(rig.bin.join(program("claude")).exists());
    assert!(rig.calls.lock().unwrap().registered.is_empty());
    write_atomic(&rig.home.join("breaker-stops.json"), b"not a stop record").unwrap();
    assert!(
        rig.manager
            .start(rig.hire())
            .unwrap_err()
            .to_string()
            .contains("damaged")
    );
}

fn failed_started_record() {
    let mut rig = Rig::new(true, true);
    let generation = rig.start();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped && rig.manager.drained());
    assert!(
        rig.problems
            .iter()
            .any(|problem| problem.contains("lifecycle record was not acknowledged"))
    );
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.registered, [generation]);
    assert_eq!(calls.revoked, [generation]);
    assert_eq!(calls.ended, [generation]);
    assert!(
        rig.problems
            .iter()
            .filter(|problem| problem.contains("lifecycle record was not acknowledged"))
            .count()
            > 1
    );
}

fn failed_requested_record() {
    let armed = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&armed);
    let mut rig = Rig::with_agora(true, false, |root, prompts, hooks| {
        Agora::open_with_book_faults(root, prompts, hooks, move |point| {
            if point == AppendPoint::BeforeWrite && trigger.swap(false, Ordering::SeqCst) {
                return Err("fixture stopped before request write".into());
            }
            Ok(())
        })
        .unwrap()
        .agora
    });
    rig.agora.committer().drained();
    armed.store(true, Ordering::SeqCst);
    let generation = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::Refused(_) | Lifecycle::Running));
    assert!(
        matches!(rig.state(), Lifecycle::Refused(reason) if reason.contains("fixture stopped before request write"))
    );
    rig.drive_until(|rig| rig.manager.drained());
    assert_eq!(rig.manager.state(&rig.id).unwrap().0, generation);
    assert!(!armed.load(Ordering::SeqCst));
    assert!(rig.calls.lock().unwrap().registered.is_empty());
    assert!(rig.output.is_empty());
    assert!(
        matches!(rig.agora.registry(), Loaded::Present(registry) if !registry.agents.contains_key(&rig.id))
    );
    assert!(
        !rig.agora
            .tail(30)
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.event == "requested" || entry.event == "started")
    );
}

fn started_record_failed_after_sync() {
    let failed = Arc::new(AtomicBool::new(false));
    let triggered = Arc::clone(&failed);
    let mut rig =
        Rig::with_agora(true, false, |root, prompts, hooks| {
            let log = root.join("log.jsonl");
            Agora::open_with_book_faults(root, prompts, hooks, move |point| {
            if point == AppendPoint::AfterWrite && !triggered.load(Ordering::SeqCst) {
                let bytes = fs::read_to_string(&log).map_err(|error| error.to_string())?;
                if let Some(line) = bytes.lines().last()
                    && matches!(read_line(line), LineRead::Entry(entry) if entry.event == "started")
                {
                    triggered.store(true, Ordering::SeqCst);
                    return Err("fixture stopped after started sync".into());
                }
            }
            Ok(())
        }).unwrap().agora
        });
    let subscriber = rig.agora.subscribe();
    let generation = rig.start();
    rig.drive_until(|rig| {
        assert_ne!(rig.state(), &Lifecycle::Running);
        rig.state() == &Lifecycle::Stopped && rig.manager.drained()
    });
    assert!(failed.load(Ordering::SeqCst));
    assert!(
        rig.problems
            .iter()
            .any(|problem| problem.contains("fixture stopped after started sync"))
    );
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.registered, [generation]);
    assert_eq!(calls.revoked, [generation]);
    assert_eq!(calls.ended, [generation]);
    let book = rig.agora.tail(40).unwrap();
    let request = book
        .entries
        .iter()
        .find(|entry| entry.event == "requested")
        .unwrap();
    let mut started = book.entries.iter().filter(|entry| entry.event == "started");
    let durable = started.next().unwrap();
    assert!(started.next().is_none());
    assert_eq!(durable.source.as_deref(), Some(rig.id.as_str()));
    assert_eq!(durable.data["attempt"].as_u64(), Some(request.seq));
    assert!(
        book.entries.iter().any(|entry| entry.event == "observed"
            && entry.data["attempt"].as_u64() == Some(request.seq))
    );
    assert!(book.entries.iter().any(
        |entry| entry.event == "unwound" && entry.data["attempt"].as_u64() == Some(request.seq)
    ));
    assert!(subscriber.try_iter().all(|entry| entry.event != "started"));
}

fn isolated_respawn() {
    let mut rig = Rig::new(true, false);
    rig.target_repo();
    let trace = rig.bin.join("trace-version");
    write_atomic(&trace, b"").unwrap();
    let mut hire = rig.hire();
    hire.isolated = true;
    let first = rig.manager.start(hire).unwrap();
    rig.drive_until_with_timeout(Duration::from_secs(60), |rig| {
        let state = rig.state();
        assert!(
            !matches!(state, Lifecycle::Refused(_)),
            "the initial isolated launch was refused: {state:?}"
        );
        state == &Lifecycle::Running
    });
    let checkout = rig.home.join("worktrees").join(rig.id.as_str());
    let probes = fs::read_to_string(trace).unwrap();
    assert_eq!(
        probes.lines().map(PathBuf::from).collect::<Vec<_>>(),
        [
            fs::canonicalize(&rig.target).unwrap(),
            fs::canonicalize(&checkout).unwrap()
        ]
    );
    let Loaded::Present(registry) = rig.agora.registry() else {
        unreachable!("missing fixture registry")
    };
    assert_eq!(
        registry.agents[&rig.id].cwd,
        fs::canonicalize(&checkout).unwrap().display().to_string()
    );
    let work = checkout.join("retained.txt");
    write_atomic(&work, b"unfinished").unwrap();
    rig.manager.kill(&rig.id, first).unwrap();
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited);
    assert_eq!(fs::read(&work).unwrap(), b"unfinished");
    assert!(
        rig.problems
            .iter()
            .any(|problem| problem.contains("uncommitted change(s); left in place"))
    );
    assert!(!rig.target.join("retained.txt").exists());
    assert_ne!(rig.manager.respawn(&rig.id, false).unwrap(), first);
    let (second, _) = rig.manager.state(&rig.id).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    assert!(rig.manager.accepted(&rig.id, first).is_err());
    rig.manager.cancel(&rig.id, second).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped);
    assert_eq!(fs::read(&work).unwrap(), b"unfinished");
    assert_eq!(rig.calls.lock().unwrap().registered, [first, second]);
}

fn checkout_returned_after_cancellation() {
    let mut rig = Rig::new(true, false);
    rig.target_repo();
    write_atomic(&rig.bin.join("hold-checkout-probe"), b"hold").unwrap();
    let mut hire = rig.hire();
    hire.isolated = true;
    let generation = rig.manager.start(hire).unwrap();
    let ready = rig.bin.join("checkout-probe-ready");
    rig.drive_until(|rig| {
        assert_eq!(rig.state(), &Lifecycle::Checking);
        ready.is_file()
    });

    let checkout = rig.home.join("worktrees").join(rig.id.as_str());
    assert_eq!(
        PathBuf::from(fs::read_to_string(ready).unwrap()),
        fs::canonicalize(&checkout).unwrap()
    );
    assert!(checkout.join(".git").is_file());
    rig.manager.cancel(&rig.id, generation).unwrap();
    assert!(!rig.manager.drained());
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    write_atomic(&rig.bin.join("release-checkout-probe"), b"release").unwrap();

    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped && rig.manager.drained());
    assert_eq!(
        fs::read(rig.bin.join("checkout-probe-released")).unwrap(),
        b"released"
    );
    assert_eq!(rig.manager.state(&rig.id).unwrap().0, generation);
    assert!(!checkout.exists());
    let worktrees = Git::target()
        .without_machine_config()
        .run(&rig.target, &["worktree", "list", "--porcelain"]);
    assert!(worktrees.ok(), "{worktrees:?}");
    assert_eq!(
        worktrees
            .stdout
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        1
    );

    let calls = rig.calls.lock().unwrap();
    assert!(calls.registered.is_empty());
    assert!(calls.ended.is_empty());
    assert!(rig.output.is_empty());
    let book = rig.agora.tail(30).unwrap();
    let requested = book
        .entries
        .iter()
        .find(|entry| entry.event == "requested")
        .unwrap();
    assert!(book.entries.iter().any(|entry| {
        entry.event == "unwound"
            && entry.source.as_deref() == Some(rig.id.as_str())
            && entry.data["attempt"].as_u64() == Some(requested.seq)
            && entry.data["checkout"]["kind"].as_str() == Some("removed")
    }));
    assert!(
        !book
            .entries
            .iter()
            .any(|entry| entry.event == "started" || entry.event == "returned")
    );
}

fn overlapping_registry_and_ledger_edits() {
    let mut rig = Rig::new(true, false);
    let second_id: AgentId = "agent.second-fixture".parse().unwrap();
    let editor_id: AgentId = "agent.editor-fixture".parse().unwrap();
    let mut editor_row = rig.hire().roster;
    editor_row.name = "Concurrent roster edit".into();
    let editor_expected = editor_row.clone();
    let editor_key = editor_id.clone();
    let agora = Arc::clone(&rig.agora);
    let (entered, ready) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let editor = thread::spawn(move || {
        agora
            .edit_registry(|registry| {
                let _old = registry.agents.insert(editor_key, editor_row);
                entered.send(()).unwrap();
                hold.recv_timeout(Duration::from_secs(15)).unwrap();
                Ok(())
            })
            .unwrap();
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let first = rig.start();
    let second = rig
        .manager
        .start(fixture_hire::hire(&second_id, &rig.target))
        .unwrap();
    rig.drive_until(|rig| {
        let book = rig.agora.tail(20).unwrap();
        [&rig.id, &second_id].into_iter().all(|id| {
            book.entries.iter().any(|line| {
                line.event == "requested" && line.source.as_deref() == Some(id.as_str())
            })
        })
    });
    assert_eq!(
        rig.manager.state(&rig.id),
        Some((first, &Lifecycle::Checking))
    );
    assert_eq!(
        rig.manager.state(&second_id),
        Some((second, &Lifecycle::Checking))
    );
    release.send(()).unwrap();
    editor.join().unwrap();
    rig.drive_until(|rig| {
        [&rig.id, &second_id]
            .into_iter()
            .all(|id| matches!(rig.manager.state(id), Some((_, Lifecycle::Running))))
    });
    let Loaded::Present(registry) = rig.agora.registry() else {
        unreachable!()
    };
    let cwd = fs::canonicalize(&rig.target).unwrap();
    assert_eq!(registry.agents[&editor_id], editor_expected);
    assert_eq!(Path::new(&registry.agents[&rig.id].cwd), cwd);
    assert_eq!(Path::new(&registry.agents[&second_id].cwd), cwd);

    let now = Timestamp::now();
    let mut ledger = Tasks::empty();
    ledger.tasks = vec![
        fixture_task(now, "11111111", TaskStatus::InProgress, rig.id.clone()),
        fixture_task(now, "22222222", TaskStatus::InProgress, second_id.clone()),
    ];
    let _seed = rig.agora.write_tasks(&ledger).unwrap();
    rig.manager.kill(&rig.id, first).unwrap();
    rig.manager.kill(&second_id, second).unwrap();
    ledger.tasks[0].title = "Edited while both exits were pending".into();
    ledger.tasks[1].spec = "Keep the operator's latest instructions".into();
    ledger.tasks.push(fixture_task(
        now,
        "33333333",
        TaskStatus::InProgress,
        editor_id,
    ));
    let _edited = rig.agora.write_tasks(&ledger).unwrap();
    rig.drive_until(|rig| {
        rig.manager.state(&rig.id) == Some((first, &Lifecycle::Exited))
            && rig.manager.state(&second_id) == Some((second, &Lifecycle::Exited))
            && rig.manager.drained()
    });

    let Loaded::Present(saved) = rig.agora.tasks() else {
        unreachable!()
    };
    assert_eq!(saved.tasks.len(), 3);
    assert_eq!(saved.tasks[0].title, ledger.tasks[0].title);
    assert_eq!(saved.tasks[1].spec, ledger.tasks[1].spec);
    assert_eq!(saved.tasks[2], ledger.tasks[2]);
    let book = rig.agora.tail(40).unwrap();
    for (before, after) in ledger.tasks[..2].iter().zip(&saved.tasks[..2]) {
        assert_eq!(after.id, before.id);
        assert_eq!(after.assignee, before.assignee);
        assert_eq!(after.status, TaskStatus::Todo);
        let task_id = after.id.to_string();
        let hire = after.assignee.as_ref().unwrap();
        assert!(book.entries.iter().any(|line| {
            line.event == "returned"
                && line.source.as_deref() == Some(hire.as_str())
                && line.data["tasks"].as_array().is_some_and(|tasks| {
                    tasks.len() == 1 && tasks[0].as_str() == Some(task_id.as_str())
                })
        }));
    }
}

fn stop_during_preparation() {
    let mut rig = Rig::new(true, false);
    rig.manager = Manager::new(
        &rig.home,
        Arc::clone(&rig.agora),
        AdapterRegistry::production(Claude::new(rig.bin.join(program("eph-hook")))),
        Git::target().without_machine_config(),
        env::vars_os()
            .filter(|(name, _)| !name.as_encoded_bytes().eq_ignore_ascii_case(b"PATH"))
            .chain([("PATH".into(), rig.bin.as_os_str().to_owned())])
            .collect(),
        TerminalSize {
            columns: 90,
            rows: 30,
        },
        Ports {
            calls: Arc::clone(&rig.calls),
            fail_book: None,
            stop_file: Some(rig.home.join("breaker-stops.json")),
        },
    )
    .unwrap();
    let generation = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::Refused(_)));
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.registered, [generation]);
    assert_eq!(calls.revoked, [generation]);
    assert!(calls.ended.is_empty());
    assert!(rig.output.is_empty());
}

fn retry_reserved_cleanup() {
    let mut rig = Rig::new(true, false);
    let first = rig.start();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    rig.calls.lock().unwrap().fail_next_revoke = true;
    rig.manager.kill(&rig.id, first).unwrap();
    rig.drive_until(|rig| {
        rig.state() == &Lifecycle::Cleaning
            && rig.manager.next_deadline(Instant::now()).is_none()
            && rig
                .problems
                .iter()
                .any(|error| error.contains("could not revoke this launch"))
    });
    assert!(!rig.manager.drained());
    assert!(rig.problems.iter().all(|problem| !problem.contains(TOKEN)));
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    let second_id: AgentId = "agent.second-fixture".parse().unwrap();
    let mut second_hire = rig.hire();
    second_hire.id = second_id.clone();
    let second = rig.manager.start(second_hire).unwrap();
    rig.drive_until(|rig| {
        rig.manager
            .state(&second_id)
            .is_some_and(|(_, state)| *state == Lifecycle::Running)
    });
    assert!(matches!(
        rig.manager.retry_cleanup(&rig.id, second),
        Err(Error::Busy(_))
    ));
    rig.manager.retry_cleanup(&rig.id, first).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited);
    assert!(
        rig.manager
            .state(&second_id)
            .is_some_and(|(_, state)| *state == Lifecycle::Running)
    );
    rig.manager.shutdown();
    rig.drive_until(|rig| rig.manager.drained());
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.revoked, [first, second]);
    assert_eq!(calls.ended, [first, second]);
}

fn capacity_and_stop() {
    let mut rig = Rig::new(true, false);
    let mut hire = rig.hire();
    hire.exit_policy = ExitPolicy::Crew;
    let first = rig.manager.start(hire).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    rig.manager
        .hold_capacity(&rig.id, first, "fixture capacity".into())
        .unwrap();
    rig.manager.kill(&rig.id, first).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    assert!(rig.manager.next_deadline(Instant::now()).is_none());
    let before = rig.agora.tail(20).unwrap();
    assert!(
        before
            .entries
            .iter()
            .any(|line| line.event == "deferred" && line.data["rung"].as_u64() == Some(0))
    );
    rig.manager
        .release_capacity(&rig.id, first, Instant::now())
        .unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    let (second, _) = rig.manager.state(&rig.id).unwrap();
    assert_ne!(first, second);
    assert!(
        rig.manager
            .release_capacity(&rig.id, first, Instant::now())
            .is_err()
    );
    rig.manager.kill(&rig.id, second).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    let after = rig.agora.tail(30).unwrap();
    assert!(after.entries.iter().any(|line| line.event == "scheduled"
        && line.data["rung"].as_u64() == Some(1)
        && line.data["wait_ms"].as_u64() == Some(5_000)));
    write_atomic(
        &rig.home.join("breaker-stops.json"),
        b"damaged while waiting",
    )
    .unwrap();
    let _events = rig.manager.drive(Instant::now() + Duration::from_secs(6));
    rig.drive_until(|rig| rig.manager.drained());
    assert_eq!(
        rig.manager.state(&rig.id),
        Some((second, &Lifecycle::Exited))
    );
    let book = rig.agora.tail(30).unwrap();
    assert!(book.entries.iter().any(|line| {
        line.event == "blocked"
            && line.data["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("damaged"))
    }));
    assert_eq!(rig.calls.lock().unwrap().registered, [first, second]);
}

fn install_failure_on_respawn() {
    let mut rig = Rig::new(true, false);
    copy_executable(&env::current_exe().unwrap(), &rig.bin.join(program("npm")));
    write_atomic(&rig.home.join("engines/fail-install"), b"yes").unwrap();
    let mut hire = rig.hire();
    hire.exit_policy = ExitPolicy::Crew;
    let first = rig.manager.start(hire).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    rig.manager.kill(&rig.id, first).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    fs::remove_file(rig.bin.join(program("claude"))).unwrap();
    let _events = rig.manager.drive(Instant::now() + Duration::from_secs(6));
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::InstallOffered(_)));
    let (second, _) = rig.manager.state(&rig.id).unwrap();
    assert_ne!(first, second);
    rig.manager.accept_install(&rig.id, second).unwrap();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::Refused(_)) && rig.manager.drained());
    let Lifecycle::Refused(reason) = rig.state() else {
        unreachable!()
    };
    assert!(
        rig.agora
            .tail(30)
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.event == "failed" && entry.data["reason"].as_str() == Some(reason))
    );
    assert!(rig.bin.join(program("claude")).exists());
    assert!(rig.manager.next_deadline(Instant::now()).is_none());
    let _events = rig.manager.drive(Instant::now() + Duration::from_secs(60));
    assert_eq!(rig.manager.state(&rig.id).unwrap().0, second);
    assert_eq!(rig.calls.lock().unwrap().registered, [first]);
}

fn auth_and_tools() {
    let mut rig = Rig::new(true, false);
    let mut hire = rig.hire();
    hire.tools.push(DeclaredTool {
        root: ToolRoot::Target,
        path: "fixture-tool".into(),
    });
    let private = ConfigDir::for_agent(&rig.home, hire.roster.engine, &rig.id).unwrap();
    fs::create_dir_all(private.path()).unwrap();
    let logout = private.path().join("fixture-logout");
    write_atomic(&logout, b"yes").unwrap();
    let first = rig.manager.start(hire).unwrap();
    rig.drive_until(|rig| {
        matches!(
            rig.state(),
            Lifecycle::NeedsLogin(_) | Lifecycle::Refused(_) | Lifecycle::Running
        )
    });
    assert!(
        matches!(rig.state(), Lifecycle::NeedsLogin(command) if command.program == "claude" && command.args == ["auth", "login"])
    );
    rig.drive_until(|rig| rig.manager.drained());
    assert!(matches!(
        rig.manager.respawn(&rig.id, true),
        Err(Error::Busy(_))
    ));
    assert!(rig.calls.lock().unwrap().registered.is_empty());
    let tool = fs::canonicalize(&rig.target).unwrap().join("fixture-tool");
    let detail = format!("declared tools missing: {}", tool.display());
    assert_eq!(rig.degradation("tools").as_deref(), Some(detail.as_str()));
    assert!(!rig.home.join("tools").exists());

    fs::remove_file(&logout).unwrap();
    fs::create_dir(&tool).unwrap();
    let mut hire = rig.hire();
    hire.exit_policy = ExitPolicy::Crew;
    hire.tools.push(DeclaredTool {
        root: ToolRoot::Target,
        path: "fixture-tool".into(),
    });
    let second = rig.manager.start(hire).unwrap();
    assert_ne!(first, second);
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    assert_eq!(rig.degradation("tools"), None);
    assert_eq!(rig.degradation("auth"), None);
    rig.manager.kill(&rig.id, second).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());

    write_atomic(&logout, b"yes").unwrap();
    let _events = rig.manager.drive(Instant::now() + Duration::from_secs(6));
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::NeedsLogin(_)) && rig.manager.drained());
    assert!(rig.agora.tail(30).unwrap().entries.iter().any(|entry| {
        entry.event == "failed" && entry.data["reason"].as_str() == Some("the engine needs login")
    }));
    assert_eq!(rig.calls.lock().unwrap().registered, [second]);
}

fn malformed_registration() {
    let mut rig = Rig::new(true, false);
    let mut generations = Vec::new();
    for (token, endpoint) in [("", "fixture-endpoint"), (TOKEN, "")] {
        rig.calls.lock().unwrap().registration = Some(HookRegistration {
            token: token.into(),
            endpoint: endpoint.into(),
        });
        generations.push(rig.start());
        rig.drive_until(|rig| matches!(rig.state(), Lifecycle::Refused(_) | Lifecycle::Running));
        assert!(
            matches!(rig.state(), Lifecycle::Refused(reason) if reason == "the hook registration did not return its token and endpoint")
        );
        rig.drive_until(|rig| rig.manager.drained());
        assert!(rig.output.is_empty());
    }
    let calls = rig.calls.lock().unwrap();
    assert_eq!(calls.registered, generations);
    assert_eq!(calls.revoked, generations);
    assert!(calls.ended.is_empty());
}

fn admission_and_stops() {
    let mut rig = Rig::new(true, false);
    let mut retired = rig.hire();
    retired.roster.retired_at = Some(Timestamp::now());
    assert!(matches!(rig.manager.start(retired), Err(Error::Refused(_))));
    let mut reserved = rig.hire();
    reserved.id = "agent.ledger".parse().unwrap();
    assert!(matches!(
        rig.manager.start(reserved),
        Err(Error::Refused(_))
    ));

    rig.write_stop(rig.id.clone());
    assert!(
        matches!(rig.manager.start(rig.hire()), Err(Error::Refused(reason)) if reason.contains("has a standing stop"))
    );
    rig.write_stop("agent.peer".parse().unwrap());
    let generation = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::Running | Lifecycle::Refused(_)));
    assert_eq!(rig.state(), &Lifecycle::Running);
    rig.write_stop(rig.id.clone());
    rig.manager.kill(&rig.id, generation).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    let book = rig.agora.tail(25).unwrap();
    assert!(book.entries.iter().any(|entry| {
        entry.event == "offered"
            && entry.data["stop"]
                .as_str()
                .is_some_and(|stop| stop.contains("standing stop"))
    }));
    assert!(
        !book
            .entries
            .iter()
            .any(|entry| entry.event == "blocked" || entry.event == "scheduled")
    );

    rig.write_stop("agent.peer".parse().unwrap());
    rig.manager.shutdown();
    assert!(
        matches!(rig.manager.start(rig.hire()), Err(Error::Refused(reason)) if reason.contains("manager is shutting down"))
    );
}

fn release_after_shutdown() {
    let mut rig = Rig::new(true, false);
    let mut hire = rig.hire();
    hire.exit_policy = ExitPolicy::Crew;
    let generation = rig.manager.start(hire).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    rig.manager
        .hold_capacity(&rig.id, generation, "no capacity".into())
        .unwrap();
    rig.manager.kill(&rig.id, generation).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    assert!(rig.manager.next_deadline(Instant::now()).is_none());

    rig.manager.shutdown();
    let now = Instant::now();
    rig.manager
        .release_capacity(&rig.id, generation, now)
        .unwrap();
    assert_ne!(rig.manager.next_deadline(now), Some(now));
    let _events = rig.manager.drive(now);
    rig.drive_until(|rig| rig.manager.drained());
    assert_eq!(
        rig.manager.state(&rig.id),
        Some((generation, &Lifecycle::Exited))
    );
    assert!(rig.manager.next_deadline(Instant::now()).is_none());
    let book = rig.agora.tail(25).unwrap();
    let requested = book
        .entries
        .iter()
        .find(|entry| entry.event == "requested")
        .unwrap();
    let mut released = book
        .entries
        .iter()
        .filter(|entry| entry.event == "released");
    let release = released.next().unwrap();
    assert!(released.next().is_none());
    assert_eq!(release.source.as_deref(), Some(rig.id.as_str()));
    assert_eq!(release.data["attempt"].as_u64(), Some(requested.seq));
    assert_eq!(release.data["rung"].as_u64(), Some(0));
    assert!(!book.entries.iter().any(|entry| entry.event == "blocked"));
}

fn resume_observed_session() {
    let mut rig = Rig::new(true, false);
    let first = rig.start();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    rig.manager
        .observe_session(&rig.id, first, "fixture-session".into())
        .unwrap();
    rig.manager
        .observe_session(&rig.id, first, String::new())
        .unwrap();
    rig.manager.kill(&rig.id, first).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());
    assert!(rig.agora.tail(25).unwrap().entries.iter().any(|entry| {
        entry.event == "offered" && entry.data["resumable"].as_bool() == Some(true)
    }));

    rig.output.clear();
    let second = rig.manager.respawn(&rig.id, true).unwrap();
    rig.drive_until(|rig| {
        rig.state() == &Lifecycle::Running
            && rig
                .text(TerminalKind::Engine)
                .contains("fixture-resume=fixture-session")
    });
    assert_eq!(
        rig.calls.lock().unwrap().transcripts_checked,
        ["fixture-session"]
    );
    rig.manager
        .observe_session(&rig.id, second, "removed-session".into())
        .unwrap();
    rig.manager.kill(&rig.id, second).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited && rig.manager.drained());

    rig.calls.lock().unwrap().transcript_present = Some(false);
    rig.output.clear();
    let third = rig.manager.respawn(&rig.id, true).unwrap();
    rig.drive_until(|rig| {
        rig.state() == &Lifecycle::Running
            && rig
                .text(TerminalKind::Engine)
                .contains("manager-fixture-ready")
    });
    rig.manager.cancel(&rig.id, third).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped && rig.manager.drained());
    assert!(!rig.text(TerminalKind::Engine).contains("fixture-resume="));
    assert_eq!(
        rig.calls.lock().unwrap().transcripts_checked,
        ["fixture-session", "removed-session"]
    );
    assert!(
        rig.problems
            .iter()
            .any(|problem| problem == "the recorded session has no transcript; starting fresh")
    );
}

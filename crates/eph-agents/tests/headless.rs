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
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eph_agents::{
    CommandPhase, Error, Event, ExitPolicy, Generation, Hire, HookRegistration, Lifecycle, Manager,
    Services, TerminalSize,
};
use eph_agora::{agora::Agora, store::Loaded};
use eph_engines::{AdapterRegistry, Claude};
use eph_fs::atomic::write_atomic;
use eph_git::{committer::Hooks, runner::Git};
use eph_schema::{
    gates::Autonomy,
    id::TaskId,
    lifecycle::TerminalKind,
    names::{AgentId, EnvName},
    refs::Ref,
    registry::{Agent, EngineId, HireRef},
    tasks::{Artifacts, Source, Task, TaskStatus, Tasks},
    time::Timestamp,
};

const TOKEN: &str = "fixture-session-secret";
const GRANT: &str = "fixture-granted-secret";
static NEXT: AtomicU64 = AtomicU64::new(0);

fn main() -> ExitCode {
    let exe = env::current_exe().unwrap();
    match exe.file_stem().and_then(|value| value.to_str()) {
        Some("claude") => return engine(),
        Some("npm") => return install(&exe),
        _ => {}
    }
    let args: Vec<_> = env::args().skip(1).collect();
    let filter = args.iter().find(|arg| !arg.starts_with('-'));
    let tests: [(&str, fn()); 9] = [
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
            "an_isolated_respawn_keeps_dirty_work_and_rejects_stale_events",
            isolated_respawn,
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
    ];
    let mut ran = 0;
    for (name, test) in tests {
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
        Some("--version") => println!("2.1.295 (Claude Code)"),
        Some("auth") => {
            let logged_in = env::var_os("CLAUDE_CONFIG_DIR").is_some()
                && env::var_os("EPH_HOOK_TOKEN").is_none()
                && env::var_os("FIXTURE_GRANT").is_none();
            println!(r#"{{"loggedIn":{logged_in}}}"#);
        }
        _ => {
            println!("manager-fixture-ready");
            println!("token={}", env::var("EPH_HOOK_TOKEN").unwrap_or_default());
            println!("grant={}", env::var("FIXTURE_GRANT").unwrap_or_default());
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

fn install(exe: &Path) -> ExitCode {
    let cwd = env::current_dir().unwrap();
    assert_eq!(cwd.file_name().unwrap(), "engines");
    assert!(env::var_os("EPH_HOOK_TOKEN").is_none());
    assert!(env::var_os("FIXTURE_GRANT").is_none());
    assert!(env::var_os("CLAUDE_CONFIG_DIR").is_none());
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

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = fs::canonicalize(env::temp_dir())
            .unwrap()
            .join(format!("ephesus-manager-{now}-{id}"));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let Ok(resolved) = fs::canonicalize(&self.0) else {
            return;
        };
        if resolved != self.0 {
            eprintln!("retained redirected headless fixture {}", self.0.display());
            return;
        }
        if let Err(error) = fs::remove_dir_all(resolved) {
            eprintln!("clean headless fixture {}: {error}", self.0.display());
        }
    }
}

#[derive(Default)]
struct Calls {
    registered: Vec<Generation>,
    revoked: Vec<Generation>,
    ended: Vec<Generation>,
    grants: Vec<String>,
    fail_next_revoke: bool,
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
        Ok(HookRegistration {
            token: TOKEN.into(),
            endpoint: "fixture-endpoint".into(),
        })
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
    _scratch: Scratch,
}

impl Rig {
    fn new(installed: bool, fail_book: bool) -> Self {
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
        let agora = Arc::new(
            Agora::open(&home.join("agora"), &home.join("prompts"), Hooks::none())
                .unwrap()
                .agora,
        );
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
            _scratch: scratch,
        }
    }

    fn hire(&self) -> Hire {
        Hire {
            id: self.id.clone(),
            roster: Agent {
                name: "Fixture hire".into(),
                role: "Test role".into(),
                engine: EngineId::Claude,
                capabilities: Vec::new(),
                seat: "fixture".into(),
                env_grants: vec!["FIXTURE_GRANT".parse().unwrap()],
                profile: None,
                cwd: self.target.display().to_string(),
                target: None,
                budget: None,
                hire: HireRef {
                    template: "fixture".parse().unwrap(),
                    version: 1,
                },
                spawned_at: Timestamp::now(),
                retired_at: None,
            },
            target: self.target.clone(),
            isolated: false,
            autonomy: Autonomy::Manual,
            identity: "Identity for the headless test.".into(),
            exit_policy: ExitPolicy::Offer,
            tools: Vec::new(),
            playbooks: None,
            unattended: Vec::new(),
            commit_identity: None,
            recall_command: None,
            credential_refresh: None,
        }
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

fn ordinary_launch() {
    let mut rig = Rig::new(true, false);
    let now = Timestamp::now();
    let task = |suffix: &str, status, assignee| Task {
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
    };
    let mut ledger = Tasks::empty();
    ledger.tasks = vec![
        task("11111111", TaskStatus::InProgress, rig.id.clone()),
        task("22222222", TaskStatus::Review, rig.id.clone()),
        task(
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
    rig.manager.cancel(&rig.id, stale).unwrap();
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped);
    assert!(!rig.home.join("worktrees").exists());
    let current = rig.start();
    rig.drive_until(|rig| matches!(rig.state(), Lifecycle::InstallOffered(_)));
    assert!(matches!(
        rig.manager.accept_install(&rig.id, stale),
        Err(Error::Busy(_))
    ));
    rig.manager.accept_install(&rig.id, current).unwrap();
    assert!(rig.manager.accept_install(&rig.id, current).is_err());
    rig.drive_until(|rig| rig.state() == &Lifecycle::Running);
    assert!(
        rig.text(TerminalKind::Installer)
            .contains("manager-fixture-installed")
    );
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
    rig.drive_until(|rig| rig.state() == &Lifecycle::Stopped);
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

fn isolated_respawn() {
    let mut rig = Rig::new(true, false);
    rig.target_repo();
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
    let work = checkout.join("retained.txt");
    write_atomic(&work, b"unfinished").unwrap();
    rig.manager.kill(&rig.id, first).unwrap();
    assert!(matches!(rig.manager.start(rig.hire()), Err(Error::Busy(_))));
    rig.drive_until(|rig| rig.state() == &Lifecycle::Exited);
    assert_eq!(fs::read(&work).unwrap(), b"unfinished");
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
    assert!(rig.bin.join(program("claude")).exists());
    assert!(rig.manager.next_deadline(Instant::now()).is_none());
    let _events = rig.manager.drive(Instant::now() + Duration::from_secs(60));
    assert_eq!(rig.manager.state(&rig.id).unwrap().0, second);
    assert_eq!(rig.calls.lock().unwrap().registered, [first]);
}

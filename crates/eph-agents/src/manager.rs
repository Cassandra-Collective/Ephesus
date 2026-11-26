use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use eph_agora::{
    agora::Agora,
    store::{Loaded, OnDamaged, Store, StoreError},
};
use eph_engines::{
    AdapterRegistry, CommandSpec, Environment, VersionCheck, assert_autonomy, base_environment,
    require_reference_engine,
};
use eph_git::{runner::Git, worktrees::Worktrees};
use eph_pty::{ExitStatus, Output, Session, SpawnPlan as PtyPlan};
use eph_schema::{
    SchemaError,
    degradation::{Cause, Source},
    id::TaskId,
    lifecycle::{
        CheckoutResult, InstallCommand, InstallVerification, LifecycleEvent, LifecycleRecord,
        ProcessResult, ReturnReason, TerminalKind,
    },
    names::AgentId,
    restart::BreakerStops,
    time::Timestamp,
};

use crate::{
    CommandPhase, Error, Event, ExitPolicy, Generation, HeldCommand, Hire, Lifecycle, Services,
    commands::{Commands, SubmitKey},
    journal::{Journal, Notice},
    launch::{self, Checked, LaunchInputs, Prepared, Runtime, Setup},
    respawn::{Due, Ladder},
};

const POLL: Duration = Duration::from_millis(5);
const OUTPUT_PER_TURN: usize = 8;

/// Geometry supplied to both installer and agent terminals.
pub struct TerminalSize {
    /// Width in terminal cells.
    pub columns: u16,
    /// Height in terminal cells.
    pub rows: u16,
}

/// The single owner of agent launches. Feed current observations and call
/// `drive` at or before its next deadline while any hire is active.
pub struct Manager<S> {
    runtime: Arc<Runtime>,
    services: S,
    geometry: TerminalSize,
    agents: BTreeMap<AgentId, Slot>,
    next: u64,
    stopping: bool,
    sender: Sender<Completion>,
    completed: Receiver<Completion>,
    journal: Journal,
    notices: Receiver<Notice>,
    events: Vec<Event>,
}

struct Slot {
    hire: Arc<Hire>,
    generation: Generation,
    attempt: Option<NonZeroU64>,
    state: Lifecycle,
    target: Option<PathBuf>,
    checkout: Option<PathBuf>,
    working: bool,
    registered: bool,
    terminal: Option<Terminal>,
    commands: Commands,
    ladder: Ladder,
    session_id: Option<String>,
    resume: bool,
    rung: Option<usize>,
    ran: bool,
    task_return: bool,
    memory: bool,
    cancelled: bool,
    pending_end: Option<Lifecycle>,
    outcome: Option<ProcessResult>,
    install_attempt: bool,
    teardown: Vec<String>,
}

impl Slot {
    fn new(
        hire: Arc<Hire>,
        generation: Generation,
        ladder: Ladder,
        session_id: Option<String>,
        rung: Option<usize>,
        resume: bool,
    ) -> Self {
        Self {
            hire,
            generation,
            attempt: None,
            state: Lifecycle::Checking,
            target: None,
            checkout: None,
            working: false,
            registered: false,
            terminal: None,
            commands: Commands::default(),
            ladder,
            session_id,
            resume,
            rung,
            ran: false,
            task_return: false,
            memory: false,
            cancelled: false,
            pending_end: None,
            outcome: None,
            install_attempt: false,
            teardown: Vec::new(),
        }
    }

    fn reserved(&self) -> bool {
        self.working
            || self.terminal.is_some()
            || self.registered
            || self.checkout.is_some()
            || matches!(
                self.state,
                Lifecycle::Checking | Lifecycle::InstallOffered(_) | Lifecycle::Cleaning
            )
    }
}

struct Terminal {
    kind: TerminalKind,
    pty: Session,
    end: Option<ProcessResult>,
    eof: bool,
    warning: Option<String>,
    input: Option<InputKind>,
    interrupt: Option<Vec<u8>>,
}

enum InputKind {
    Text { arm_submit: bool },
    Key,
    Interrupt,
    Installer,
}

impl Terminal {
    fn new(kind: TerminalKind, pty: Session) -> Self {
        Self {
            kind,
            pty,
            end: None,
            eof: false,
            warning: None,
            input: None,
            interrupt: None,
        }
    }

    fn writable(&self) -> bool {
        self.input.is_none() && self.interrupt.is_none()
    }

    fn enqueue(&mut self, bytes: Vec<u8>, kind: InputKind) -> Result<(), Error> {
        self.pty.queue_write(bytes)?;
        self.input = Some(kind);
        Ok(())
    }

    fn disarm_text(&mut self) {
        if let Some(InputKind::Text { arm_submit }) = &mut self.input {
            *arm_submit = false;
        }
    }
}

struct Completion {
    agent: AgentId,
    generation: Generation,
    result: Effect,
}

enum Effect {
    Version(Option<NonZeroU64>, Result<(PathBuf, VersionCheck), Error>),
    InstallerReady(Result<InstallerReady, Error>),
    Installed(Result<eph_engines::VerifiedExecutable, Error>),
    Setup(Box<Setup>),
    Prepared(Box<Result<Prepared, Error>>),
    Started(Result<(), Error>, bool),
    Released(CheckoutResult, Option<Result<Vec<TaskId>, StoreError>>),
}

struct InstallerReady {
    executable: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    environment: Environment,
}

impl<S: Services> Manager<S> {
    /// Wire the current production adapter registry to its home and Agora.
    pub fn new(
        home: &Path,
        agora: Arc<Agora>,
        adapters: AdapterRegistry,
        git: Git,
        parent: Environment,
        geometry: TerminalSize,
        services: S,
    ) -> Result<Self, Error> {
        let home = fs::canonicalize(home)?;
        let agora_root = fs::canonicalize(agora.root())?;
        launch::owned_dir(&agora_root, Path::new(".git"), false)?;
        let worktrees = Worktrees::new(git, &agora_root);
        let (sender, completed) = mpsc::channel();
        let (journal, notices) = Journal::open(Arc::clone(&agora))?;
        Ok(Self {
            runtime: Arc::new(Runtime {
                home,
                agora,
                agora_root,
                adapters,
                worktrees,
                parent,
            }),
            services,
            geometry,
            agents: BTreeMap::new(),
            next: 0,
            stopping: false,
            sender,
            completed,
            journal,
            notices,
            events: Vec::new(),
        })
    }

    /// Request a launch after admission checks. A pending effect retains the
    /// hire's reservation until a subsequent drive completes it.
    pub fn start(&mut self, hire: Hire) -> Result<Generation, Error> {
        let id = hire.id.clone();
        if self.agents.get(&id).is_some_and(Slot::reserved) {
            return Err(Error::Busy(id));
        }
        self.begin(Arc::new(hire), Ladder::default(), None, None, false)
    }

    /// Respawn a terminated hire, with its observed session if requested.
    pub fn respawn(&mut self, agent: &AgentId, resume: bool) -> Result<Generation, Error> {
        self.with_slot(agent, |manager, slot| {
            if slot.reserved() || slot.state != Lifecycle::Exited {
                return Err(Error::Busy(agent.clone()));
            }
            manager.admit(&slot.hire)?;
            slot.ladder.manual();
            let ladder = slot.ladder.clone();
            let session = slot.session_id.clone();
            match manager.make_slot(Arc::clone(&slot.hire), ladder, session, None, resume) {
                Ok(replacement) => {
                    *slot = replacement;
                    Ok(slot.generation)
                }
                Err(error) => Err(error),
            }
        })
    }

    fn begin(
        &mut self,
        hire: Arc<Hire>,
        ladder: Ladder,
        session: Option<String>,
        rung: Option<usize>,
        resume: bool,
    ) -> Result<Generation, Error> {
        self.admit(&hire)?;
        let slot = self.make_slot(hire, ladder, session, rung, resume)?;
        let generation = slot.generation;
        self.agents.insert(slot.hire.id.clone(), slot);
        Ok(generation)
    }

    fn make_slot(
        &mut self,
        hire: Arc<Hire>,
        ladder: Ladder,
        session_id: Option<String>,
        rung: Option<usize>,
        resume: bool,
    ) -> Result<Slot, Error> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| Error::Refused("the launch sequence is exhausted".into()))?;
        let mut slot = Slot::new(
            hire,
            Generation(self.next),
            ladder,
            session_id,
            rung,
            resume,
        );
        let request = record(
            &slot,
            LifecycleEvent::Requested {
                engine: slot.hire.roster.engine,
                workspace: slot.hire.target.display().to_string(),
                grants: slot.hire.roster.env_grants.clone(),
            },
        );
        let hire = Arc::clone(&slot.hire);
        let journal = self.journal.clone();
        if let Err(error) = self.work(&mut slot, move |runtime| {
            let mut attempt = None;
            let result = (|| {
                attempt = NonZeroU64::new(journal.acknowledge(request)?);
                let target = runtime.target(&hire)?;
                persist_hire(runtime, &hire, &target)?;
                let binary = runtime.adapter(&hire)?.binary();
                let check = runtime.version(binary, &target)?;
                if check == VersionCheck::Missing {
                    let cwd = installer_cwd(runtime)?;
                    let _offer = journal.acknowledge(LifecycleRecord {
                        agent: hire.id.clone(),
                        attempt,
                        event: LifecycleEvent::InstallOffered(installer(binary.install(), &cwd)),
                    })?;
                }
                Ok((target, check))
            })();
            Effect::Version(attempt, result)
        }) {
            self.refuse(&mut slot, error.to_string());
        }
        Ok(slot)
    }

    fn admit(&self, hire: &Hire) -> Result<(), Error> {
        if self.stopping || hire.id.is_reserved() || hire.roster.retired_at.is_some() {
            return Err(Error::Refused(
                "the hire is retired, reserved, or the manager is shutting down".into(),
            ));
        }
        require_reference_engine(hire.roster.engine)
            .map_err(|error| Error::Refused(error.to_string()))?;
        assert_autonomy(self.runtime.adapter(hire)?, hire.autonomy)
            .map_err(|error| Error::Refused(error.to_string()))?;
        self.standing_stop(&hire.id)
    }

    fn standing_stop(&self, id: &AgentId) -> Result<(), Error> {
        let store = Store::<BreakerStops>::at(&self.runtime.home, OnDamaged::Block);
        match store.load() {
            Loaded::Present(stops) if stops.stops.contains_key(id) => {
                Err(Error::Refused(format!("{id} has a standing stop")))
            }
            Loaded::Damaged(damage) => Err(Error::Refused(damage.detail())),
            _ => Ok(()),
        }
    }

    /// Current generation and lifecycle for the caller's card.
    pub fn state(&self, agent: &AgentId) -> Option<(Generation, &Lifecycle)> {
        self.agents
            .get(agent)
            .map(|slot| (slot.generation, &slot.state))
    }

    /// Text still waiting for this hire to become ready.
    pub fn held(&self, agent: &AgentId) -> Option<&HeldCommand> {
        self.agents.get(agent).and_then(|slot| slot.commands.held())
    }

    /// Accept the exact current offer and open its installer terminal.
    pub fn accept_install(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.with_slot(agent, |manager, slot| {
            if slot.generation != generation || slot.cancelled {
                return Err(Error::Busy(agent.clone()));
            }
            let Lifecycle::InstallOffered(command) = slot.state else {
                return Err(Error::Refused("there is no current install offer".into()));
            };
            manager.admit(&slot.hire)?;
            let journal = manager.journal.clone();
            let attempt = slot.attempt;
            let agent = slot.hire.id.clone();
            slot.state = Lifecycle::Checking;
            slot.install_attempt = true;
            let queued = manager.work(slot, move |runtime| {
                Effect::InstallerReady((|| {
                    let cwd = installer_cwd(runtime)?;
                    let environment = base_environment(&runtime.parent);
                    let executable = launch::installer_path(command, &cwd, &environment)?;
                    let _accepted = journal.acknowledge(LifecycleRecord {
                        agent,
                        attempt,
                        event: LifecycleEvent::InstallAccepted(installer(command, &cwd)),
                    })?;
                    let args = command.args.iter().map(OsString::from).collect();
                    Ok(InstallerReady {
                        executable,
                        args,
                        cwd,
                        environment,
                    })
                })())
            });
            if let Err(error) = &queued {
                manager.refuse(slot, error.to_string());
            }
            queued
        })
    }

    /// Decline the current offer or cancel the exact current attempt.
    pub fn cancel(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.with_slot(agent, |manager, slot| {
            if slot.generation != generation {
                return Err(Error::Busy(agent.clone()));
            }
            manager.cancel_slot(slot);
            Ok(())
        })
    }

    /// Retry outstanding teardown for the exact reserved launch once its active
    /// terminal and worker have settled. A retry preserves the intended outcome.
    pub fn retry_cleanup(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.with_slot(agent, |manager, slot| {
            if slot.generation != generation || slot.working || slot.terminal.is_some() {
                return Err(Error::Busy(agent.clone()));
            }
            let state = slot.pending_end.clone().ok_or_else(|| {
                Error::Refused("the hire has no outstanding cleanup to retry".into())
            })?;
            manager.unwind(slot, state);
            Ok(())
        })
    }

    /// Whether all terminals, outstanding work, and cleanup reservations settled.
    pub fn drained(&self) -> bool {
        self.agents.values().all(|slot| !slot.reserved()) && !self.journal.pending()
    }

    fn cancel_slot(&mut self, slot: &mut Slot) {
        slot.cancelled = true;
        slot.ladder.manual();
        slot.commands.interrupt();
        if slot.pending_end.is_some() {
            slot.pending_end = Some(Lifecycle::Stopped);
        }
        if let Some(terminal) = &mut slot.terminal {
            if let Err(error) = terminal.pty.kill() {
                self.problem(&slot.hire.id, "teardown", error.to_string());
            }
            slot.state = Lifecycle::Cleaning;
        } else if !slot.working {
            self.unwind(slot, Lifecycle::Stopped);
        }
    }

    /// Cancel every outstanding attempt and keep driving until ownership settles.
    pub fn shutdown(&mut self) {
        self.stopping = true;
        let ids: Vec<_> = self.agents.keys().cloned().collect();
        for id in ids {
            if let Some(mut slot) = self.agents.remove(&id) {
                if slot.reserved() || slot.ladder.deadline().is_some() {
                    self.cancel_slot(&mut slot);
                }
                self.agents.insert(id, slot);
            }
        }
    }

    /// Earliest useful drive, including polling deadlines for active terminals and effects.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.agents
            .values()
            .flat_map(|slot| {
                [
                    slot.terminal
                        .as_ref()
                        .filter(|terminal| terminal.writable())
                        .and(slot.commands.deadline()),
                    slot.ladder.deadline(),
                    (slot.working || slot.terminal.is_some()).then_some(now + POLL),
                ]
                .into_iter()
                .flatten()
            })
            .chain(self.journal.pending().then_some(now + POLL))
            .min()
    }

    /// Process worker results, drain bounded terminal chunks, and serve due
    /// commands and respawns. Every returned event belongs to its originating hire.
    pub fn drive(&mut self, now: Instant) -> Vec<Event> {
        for notice in self.notices.try_iter() {
            self.journal.notice_received();
            self.events.push(Event::Problem {
                agent: notice.agent,
                detail: notice.detail,
            });
        }
        while let Ok(completion) = self.completed.try_recv() {
            let id = completion.agent;
            if let Some(mut slot) = self.agents.remove(&id) {
                if completion.generation == slot.generation {
                    slot.working = false;
                    self.on_effect(&mut slot, completion.result, now);
                }
                self.agents.insert(id, slot);
            }
        }
        let ids: Vec<_> = self.agents.keys().cloned().collect();
        for id in ids {
            if let Some(mut slot) = self.agents.remove(&id) {
                self.poll_terminal(&mut slot, now);
                self.submit_key(&mut slot, now);
                self.due_respawn(&mut slot, now);
                self.agents.insert(id, slot);
            }
        }
        std::mem::take(&mut self.events)
    }

    fn work(
        &self,
        slot: &mut Slot,
        effect: impl FnOnce(&Runtime) -> Effect + Send + 'static,
    ) -> Result<(), Error> {
        let agent = slot.hire.id.clone();
        let generation = slot.generation;
        let runtime = Arc::clone(&self.runtime);
        let sender = self.sender.clone();
        let _worker = thread::Builder::new()
            .name("eph-agents-effect".into())
            .spawn(move || {
                let result = effect(&runtime);
                let _delivered = sender.send(Completion {
                    agent,
                    generation,
                    result,
                });
            })?;
        slot.working = true;
        Ok(())
    }

    fn on_effect(&mut self, slot: &mut Slot, effect: Effect, now: Instant) {
        if let Effect::Setup(setup) = &effect {
            slot.checkout = setup.checkout.clone();
        }
        if let Effect::Released(checkout, returned) = effect {
            if !matches!(checkout, CheckoutResult::Unconfirmed { .. }) {
                slot.checkout = None;
            }
            if let Some(returned) = returned {
                slot.task_return = false;
                self.return_work(slot, returned);
            }
            self.finished(slot, checkout, now);
            return;
        }
        if let Effect::Started(result, memory) = effect {
            self.on_started(slot, result, memory, now);
            return;
        }
        if let Effect::Version(attempt, _) = &effect {
            slot.attempt = *attempt;
        }
        if slot.cancelled {
            self.unwind(slot, Lifecycle::Stopped);
            return;
        }
        match effect {
            Effect::Version(_, result) => match result {
                Ok((target, check)) => {
                    slot.target = Some(target);
                    self.on_version(slot, check);
                }
                Err(error) => self.refuse(slot, error.to_string()),
            },
            Effect::InstallerReady(result) => self.open_installer(slot, result),
            Effect::Installed(result) => match result {
                Ok(executable) => self.setup(slot, executable),
                Err(error) => self.refuse(slot, error.to_string()),
            },
            Effect::Setup(setup) => match setup.result {
                Ok(checked) => self.prepare(slot, checked),
                Err(error) => self.refuse(slot, error.to_string()),
            },
            Effect::Prepared(result) => match *result {
                Ok(plan) => self.launch(slot, plan, now),
                Err(error) => self.refuse(slot, error.to_string()),
            },
            Effect::Released(_, _) | Effect::Started(_, _) => {}
        }
    }

    fn open_installer(&mut self, slot: &mut Slot, ready: Result<InstallerReady, Error>) {
        let opened = ready.and_then(|ready| {
            self.admit(&slot.hire)?;
            self.open_pty(
                &ready.executable,
                &ready.args,
                &ready.cwd,
                &ready.environment,
                Vec::new(),
            )
        });
        match opened {
            Ok(pty) => {
                slot.terminal = Some(Terminal::new(TerminalKind::Installer, pty));
                slot.state = Lifecycle::Installing;
            }
            Err(error) => self.refuse(slot, error.to_string()),
        }
    }

    fn on_version(&mut self, slot: &mut Slot, check: VersionCheck) {
        if check == VersionCheck::Missing {
            match self
                .runtime
                .adapter(&slot.hire)
                .map(|adapter| adapter.binary().install())
            {
                Ok(command) => slot.state = Lifecycle::InstallOffered(command),
                Err(error) => self.refuse(slot, error.to_string()),
            }
        } else {
            match launch::verified(check) {
                Ok(executable) => self.setup(slot, executable),
                Err(error) => self.refuse(slot, error.to_string()),
            }
        }
    }

    fn setup(&mut self, slot: &mut Slot, executable: eph_engines::VerifiedExecutable) {
        let Some(target) = slot.target.clone() else {
            self.refuse(slot, "the verified target is unavailable".into());
            return;
        };
        let hire = Arc::clone(&slot.hire);
        if let Err(error) = self.work(slot, move |runtime| {
            Effect::Setup(Box::new(runtime.setup(&hire, target, executable)))
        }) {
            self.refuse(slot, error.to_string());
        }
    }

    fn prepare(&mut self, slot: &mut Slot, checked: Checked) {
        if checked.missing_tools().is_empty() {
            self.clear_problem(&slot.hire.id, "tools");
        } else {
            let missing = checked
                .missing_tools()
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            self.problem(
                &slot.hire.id,
                "tools",
                format!("declared tools missing: {}", missing.join(", ")),
            );
        }
        if let Some(login) = checked.login {
            self.report_record(
                slot,
                LifecycleEvent::Refused {
                    reason: "the reference engine needs login".into(),
                },
            );
            self.unwind(slot, Lifecycle::NeedsLogin(login));
            return;
        }
        if let Some(reason) = &checked.auth_notice {
            self.problem(&slot.hire.id, "auth", reason.clone());
        } else {
            self.clear_problem(&slot.hire.id, "auth");
        }
        let inputs = self.launch_inputs(slot);
        match inputs {
            Ok(inputs) => {
                let hire = Arc::clone(&slot.hire);
                if let Err(error) = self.work(slot, move |runtime| {
                    Effect::Prepared(Box::new(
                        persist_hire(runtime, &hire, &checked.workspace.cwd)
                            .and_then(|()| runtime.prepare(&hire, checked, inputs)),
                    ))
                }) {
                    self.refuse(slot, error.to_string());
                }
            }
            Err(error) => self.refuse(slot, error.to_string()),
        }
    }

    fn launch_inputs(&mut self, slot: &mut Slot) -> Result<LaunchInputs, Error> {
        self.admit(&slot.hire)?;
        let id = &slot.hire.id;
        let mut grants = Vec::new();
        let mut missing = Vec::new();
        for name in &slot.hire.roster.env_grants {
            match self.services.grant(id, name) {
                Ok(Some(value)) => grants.push((name.clone(), OsString::from(value))),
                Ok(None) => missing.push(name.to_string()),
                Err(_) => {
                    return Err(Error::Refused(format!(
                        "the broker could not resolve {name}"
                    )));
                }
            }
        }
        if missing.is_empty() {
            self.clear_problem(id, "grants");
        } else {
            self.problem(
                id,
                "grants",
                format!("declared grants missing: {}", missing.join(", ")),
            );
        }
        let memory = self.services.memory(id).map_err(|_| {
            Error::Refused("the memory service could not read this hire's memory".into())
        })?;
        let resume = if slot.resume {
            slot.session_id.clone().filter(|session| {
                if self.services.transcript_present(id, session) == Some(false) {
                    self.problem(
                        id,
                        "resume",
                        "the recorded session has no transcript; starting fresh".into(),
                    );
                    return false;
                }
                true
            })
        } else {
            None
        };
        let registration = self.services.register(id, slot.generation).map_err(|_| {
            Error::Refused("the hook service could not register this launch".into())
        })?;
        slot.registered = true;
        if registration.token.is_empty() || registration.endpoint.is_empty() {
            return Err(Error::Refused(
                "the hook registration did not return its token and endpoint".into(),
            ));
        }
        Ok(LaunchInputs {
            registration,
            grants,
            memory,
            resume,
        })
    }

    fn launch(&mut self, slot: &mut Slot, prepared: Prepared, now: Instant) {
        if let Some(condition) = prepared.plan.prompt_condition()
            && let Err(error) = self.journal.condition(condition.clone())
        {
            self.problem(&slot.hire.id, "launch", error.to_string());
        }
        let start = (|| {
            let mut args = prepared.plan.args().to_vec();
            args.extend(prepared.resume_args);
            self.admit(&slot.hire)?;
            self.open_pty(
                prepared.executable.path(),
                &args,
                prepared.plan.cwd(),
                prepared.plan.environment(),
                prepared.secrets,
            )
        })();
        let pty = match start {
            Ok(pty) => pty,
            Err(error) => {
                self.refuse(slot, error.to_string());
                return;
            }
        };
        slot.terminal = Some(Terminal::new(TerminalKind::Engine, pty));
        slot.ran = true;
        let event = record(
            slot,
            LifecycleEvent::Started {
                version: prepared.executable.version().into(),
                workspace: prepared.plan.cwd().display().to_string(),
            },
        );
        let journal = self.journal.clone();
        let memory = prepared.memory;
        if let Err(error) = self.work(slot, move |_| {
            Effect::Started(journal.acknowledge(event).map(|_| ()), memory)
        }) {
            self.on_started(slot, Err(error), memory, now);
        }
    }

    fn on_started(
        &mut self,
        slot: &mut Slot,
        result: Result<(), Error>,
        memory: bool,
        now: Instant,
    ) {
        if let Err(error) = result {
            self.problem(&slot.hire.id, "log", error.to_string());
            self.cancel_slot(slot);
            return;
        }
        if slot.cancelled
            || slot
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.end.is_some() || terminal.warning.is_some())
        {
            return;
        }
        slot.state = Lifecycle::Running;
        slot.memory = memory;
        slot.ladder.running(now);
        slot.session_id = None;
        if let Some(rung) = slot.rung.take() {
            self.report_record(slot, LifecycleEvent::Respawned { rung });
        }
    }

    fn open_pty(
        &self,
        exe: &Path,
        args: &[OsString],
        cwd: &Path,
        env: &[(OsString, OsString)],
        secrets: Vec<String>,
    ) -> Result<Session, Error> {
        Ok(Session::spawn_queued(
            PtyPlan {
                executable: exe.as_os_str(),
                args,
                cwd,
                environment: env,
                cols: self.geometry.columns,
                rows: self.geometry.rows,
            },
            secrets,
        )?)
    }

    /// Supply a current, authenticated engine phase.
    pub fn observe_phase(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        phase: CommandPhase,
    ) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            let writable = running(slot, generation)?.writable();
            slot.commands.observe(phase);
            if writable && let Some(text) = slot.commands.take_ready() {
                send_text(slot, generation, text)?;
            }
            Ok(())
        })
    }

    /// Report proof of a turn from the current engine. Repeated proof is safe.
    pub fn accepted(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            running(slot, generation)?.disarm_text();
            slot.commands.accepted();
            Ok(())
        })
    }

    /// Record the latest session ID seen by the current launch's hook service.
    pub fn observe_session(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        session_id: String,
    ) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            running(slot, generation)?;
            if !session_id.is_empty() {
                slot.session_id = Some(session_id);
            }
            Ok(())
        })
    }

    /// Send or hold operator text for the current agent engine.
    pub fn submit(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        text: String,
    ) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            let writable = running(slot, generation)?.writable();
            if let Some(text) = slot.commands.text(text, writable) {
                send_text(slot, generation, text)?;
            }
            Ok(())
        })
    }

    /// Send interactive bytes to the visible installer, outside the agent queue.
    pub fn write_installer(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        bytes: &[u8],
    ) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            if slot.generation != generation || slot.state != Lifecycle::Installing {
                return Err(Error::Busy(agent.clone()));
            }
            let terminal = slot
                .terminal
                .as_mut()
                .ok_or_else(|| Error::Busy(agent.clone()))?;
            terminal.enqueue(bytes.to_vec(), InputKind::Installer)?;
            Ok(())
        })
    }

    /// Clear held text and pending submit keys before interrupting the engine.
    pub fn interrupt(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.with_slot(agent, |manager, slot| {
            running(slot, generation)?;
            let bytes = manager.runtime.adapter(&slot.hire)?.interrupt().bytes;
            slot.commands.interrupt();
            let terminal = running(slot, generation)?;
            terminal.disarm_text();
            if terminal.writable() {
                terminal.enqueue(bytes.to_vec(), InputKind::Interrupt)?;
            } else {
                terminal.interrupt = Some(bytes.to_vec());
            }
            Ok(())
        })
    }

    /// Resize the exact current engine or installer terminal.
    pub fn resize(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        kind: TerminalKind,
        geometry: TerminalSize,
    ) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            if slot.generation != generation
                || !matches!(slot.state, Lifecycle::Running | Lifecycle::Installing)
            {
                return Err(Error::Busy(agent.clone()));
            }
            let terminal = slot
                .terminal
                .as_mut()
                .ok_or_else(|| Error::Busy(agent.clone()))?;
            if terminal.kind != kind {
                return Err(Error::Busy(agent.clone()));
            }
            terminal.pty.resize(geometry.columns, geometry.rows)?;
            Ok(())
        })
    }

    /// Request termination of a live engine, leaving its declared exit policy in force.
    pub fn kill(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error> {
        self.with_slot(agent, |_, slot| {
            running(slot, generation)?.pty.kill()?;
            slot.commands.interrupt();
            slot.state = Lifecycle::Cleaning;
            Ok(())
        })
    }

    /// Park a crew respawn while the capacity caller holds the hire.
    pub fn hold_capacity(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        reason: String,
    ) -> Result<(), Error> {
        self.with_slot(agent, |manager, slot| {
            if slot.generation != generation {
                return Err(Error::Busy(agent.clone()));
            }
            if slot.hire.exit_policy != ExitPolicy::Offer
                && let Some(event) = slot.ladder.hold(reason)
            {
                manager.report_record(slot, event);
            }
            Ok(())
        })
    }

    /// Release a capacity hold. An exit served by this release costs no rung.
    pub fn release_capacity(
        &mut self,
        agent: &AgentId,
        generation: Generation,
        now: Instant,
    ) -> Result<(), Error> {
        self.with_slot(agent, |manager, slot| {
            if slot.generation != generation {
                return Err(Error::Busy(agent.clone()));
            }
            if let Some(event) = slot.ladder.release(now) {
                manager.report_record(slot, event);
            }
            Ok(())
        })
    }

    fn submit_key(&mut self, slot: &mut Slot, now: Instant) {
        let generation = slot.generation;
        if !running(slot, generation).is_ok_and(|terminal| terminal.writable()) {
            return;
        }
        if let Some(text) = slot.commands.take_ready() {
            if let Err(error) = send_text(slot, generation, text) {
                self.problem(&slot.hire.id, "command", error.to_string());
            }
            return;
        }
        match slot.commands.due(now) {
            Some(SubmitKey::Press) => {
                if let Err(error) = running(slot, generation)
                    .and_then(|terminal| terminal.enqueue(b"\r".to_vec(), InputKind::Key))
                {
                    slot.commands.accepted();
                    self.problem(&slot.hire.id, "command", error.to_string());
                }
            }
            Some(SubmitKey::Unconfirmed(keys)) => self.problem(
                &slot.hire.id,
                "command",
                format!("the engine did not acknowledge a submit after {keys} keys"),
            ),
            None => {}
        }
    }

    fn poll_terminal(&mut self, slot: &mut Slot, now: Instant) {
        let Some(mut terminal) = slot.terminal.take() else {
            return;
        };
        self.poll_input(slot, &mut terminal, now);
        if terminal.end.is_none() {
            let status = terminal.pty.try_wait();
            self.observe_terminal_exit(slot, &mut terminal, status);
        }
        for _ in 0..OUTPUT_PER_TURN {
            if terminal.eof {
                break;
            }
            match terminal.pty.output(Duration::ZERO) {
                Ok(Output::Data(bytes)) => self.events.push(Event::Output {
                    agent: slot.hire.id.clone(),
                    generation: slot.generation,
                    terminal: terminal.kind,
                    bytes,
                }),
                Ok(Output::Pending) => break,
                Ok(Output::End) => terminal.eof = true,
                Err(error) => self.problem(&slot.hire.id, "terminal", error.to_string()),
            }
        }
        if terminal.eof
            && !slot.working
            && let Some(status) = terminal.end.take()
        {
            let kind = terminal.kind;
            drop(terminal);
            self.report_record(
                slot,
                LifecycleEvent::Observed {
                    terminal: kind,
                    status: status.clone(),
                },
            );
            match kind {
                TerminalKind::Installer => self.installer_exit(slot, status),
                TerminalKind::Engine => {
                    slot.outcome = Some(status);
                    slot.ladder.exited(now, slot.hire.exit_policy);
                    let state = if slot.cancelled {
                        Lifecycle::Stopped
                    } else {
                        Lifecycle::Exited
                    };
                    self.unwind(slot, state);
                }
            }
        } else {
            slot.terminal = Some(terminal);
        }
    }

    fn observe_terminal_exit(
        &mut self,
        slot: &mut Slot,
        terminal: &mut Terminal,
        status: std::io::Result<Option<ExitStatus>>,
    ) {
        match status {
            Ok(Some(status)) => {
                terminal.end = Some(process_result(status));
                slot.commands.interrupt();
                slot.state = Lifecycle::Cleaning;
            }
            Ok(None) => {}
            Err(error) => {
                slot.commands.interrupt();
                slot.state = Lifecycle::Cleaning;
                let detail = format!("terminal exit is unconfirmed: {error}");
                if terminal.warning.as_deref() != Some(&detail) {
                    self.report_record(
                        slot,
                        LifecycleEvent::Observed {
                            terminal: terminal.kind,
                            status: ProcessResult::Unconfirmed {
                                reason: detail.clone(),
                            },
                        },
                    );
                    self.problem(&slot.hire.id, "teardown", detail.clone());
                    terminal.warning = Some(detail);
                }
            }
        }
    }

    fn poll_input(&mut self, slot: &mut Slot, terminal: &mut Terminal, now: Instant) {
        if let Some(result) = terminal.pty.write_result() {
            self.acknowledge_input(slot, terminal.input.take(), result, now);
        }
        if terminal.input.is_none()
            && slot.state == Lifecycle::Running
            && let Some(bytes) = terminal.interrupt.take()
            && let Err(error) = terminal.enqueue(bytes, InputKind::Interrupt)
        {
            self.problem(&slot.hire.id, "command", error.to_string());
        }
    }

    fn acknowledge_input(
        &mut self,
        slot: &mut Slot,
        purpose: Option<InputKind>,
        result: std::io::Result<()>,
        now: Instant,
    ) {
        match result {
            Ok(())
                if matches!(purpose, Some(InputKind::Text { arm_submit: true }))
                    && slot.state == Lifecycle::Running =>
            {
                slot.commands.sent(now);
            }
            Ok(()) if matches!(purpose, Some(InputKind::Key)) => slot.commands.key_written(now),
            Ok(()) => {}
            Err(error) => {
                slot.commands.accepted();
                self.problem(&slot.hire.id, "command", error.to_string());
            }
        }
    }

    fn installer_exit(&mut self, slot: &mut Slot, status: ProcessResult) {
        if status != (ProcessResult::Code { code: 0 }) || slot.cancelled {
            self.report_record(
                slot,
                LifecycleEvent::InstallExited {
                    status,
                    verification: InstallVerification::Skipped,
                },
            );
            if slot.cancelled {
                self.unwind(slot, Lifecycle::Stopped);
            } else {
                self.refuse(slot, "the installer did not exit successfully".into());
            }
            return;
        }
        let start = (|| {
            self.admit(&slot.hire)?;
            let binary = self.runtime.adapter(&slot.hire)?.binary();
            let target = slot
                .target
                .clone()
                .ok_or_else(|| Error::Refused("the install target is unavailable".into()))?;
            let completed = status.clone();
            let journal = self.journal.clone();
            let agent = slot.hire.id.clone();
            let attempt = slot.attempt;
            self.work(slot, move |runtime| {
                let version = runtime.version(binary, &target).and_then(launch::verified);
                let verification = match &version {
                    Ok(executable) => InstallVerification::Verified {
                        version: executable.version().into(),
                    },
                    Err(error) => InstallVerification::Refused {
                        reason: error.to_string(),
                    },
                };
                let written = journal.acknowledge(LifecycleRecord {
                    agent,
                    attempt,
                    event: LifecycleEvent::InstallExited {
                        status: completed,
                        verification,
                    },
                });
                Effect::Installed(written.and(version))
            })
        })();
        if let Err(error) = start {
            self.report_record(
                slot,
                LifecycleEvent::InstallExited {
                    status,
                    verification: InstallVerification::Refused {
                        reason: error.to_string(),
                    },
                },
            );
            self.refuse(slot, error.to_string());
        }
    }

    fn refuse(&mut self, slot: &mut Slot, reason: String) {
        self.report_record(
            slot,
            LifecycleEvent::Refused {
                reason: reason.clone(),
            },
        );
        self.problem(&slot.hire.id, "launch", reason.clone());
        self.unwind(slot, Lifecycle::Refused(reason));
    }

    fn unwind(&mut self, slot: &mut Slot, state: Lifecycle) {
        slot.state = Lifecycle::Cleaning;
        slot.pending_end = Some(state);
        slot.commands.interrupt();
        if slot.registered {
            match self.services.revoke(&slot.hire.id, slot.generation) {
                Ok(()) => slot.registered = false,
                Err(_) => self.teardown_problem(slot, "The hook service could not revoke this launch. Retry cleanup for this generation.".into()),
            }
        }
        if slot.ran {
            slot.ran = false;
            slot.task_return = true;
            if self.services.ended(&slot.hire.id, slot.generation).is_err() {
                self.teardown_problem(
                    slot,
                    "the exit service could not finish this session".into(),
                );
            }
        }
        let checkout = slot.checkout.clone();
        let repo = slot.target.clone();
        let agent = slot.hire.id.clone();
        let return_tasks = slot.task_return;
        if let Err(error) = self.work(slot, move |runtime| {
            let returned = return_tasks.then(|| {
                let returned = runtime.agora.return_tasks_of(&agent, Timestamp::now())?;
                if !returned.is_empty() {
                    runtime
                        .agora
                        .committer()
                        .commit_soon("agents: return unfinished work");
                }
                Ok(returned)
            });
            Effect::Released(
                runtime.cleanup(repo.as_deref(), checkout.as_deref()),
                returned,
            )
        }) {
            let reason = error.to_string();
            self.teardown_problem(slot, reason.clone());
            self.report_record(
                slot,
                LifecycleEvent::Unwound {
                    checkout: CheckoutResult::Unconfirmed { reason },
                    failures: slot.teardown.clone(),
                },
            );
        }
    }

    fn return_work(&mut self, slot: &mut Slot, returned: Result<Vec<TaskId>, StoreError>) {
        let tasks = match returned {
            Ok(tasks) => tasks,
            Err(error) => {
                self.teardown_problem(slot, error.to_string());
                Vec::new()
            }
        };
        if !tasks.is_empty() {
            self.report_record(
                slot,
                LifecycleEvent::Returned {
                    tasks: tasks.clone(),
                    reason: ReturnReason::AgentExit,
                },
            );
        }
        let Some(status) = slot
            .outcome
            .clone()
            .filter(|_| slot.pending_end == Some(Lifecycle::Exited))
        else {
            return;
        };
        let resumable = slot.session_id.as_deref().is_some_and(|session| {
            self.runtime
                .adapter(&slot.hire)
                .is_ok_and(|adapter| adapter.resume(session).is_some())
        });
        self.report_record(
            slot,
            LifecycleEvent::Offered {
                status,
                resumable,
                memory: slot.memory,
                tasks,
                stop: self
                    .standing_stop(&slot.hire.id)
                    .err()
                    .map(|error| error.to_string()),
                capacity: slot.ladder.capacity().map(str::to_owned),
            },
        );
    }

    fn finished(&mut self, slot: &mut Slot, checkout: CheckoutResult, now: Instant) {
        match &checkout {
            CheckoutResult::Removed {
                residue: Some(reason),
            }
            | CheckoutResult::Retained { reason, .. } => {
                self.teardown_problem(slot, reason.clone());
            }
            _ => {}
        }
        self.report_record(
            slot,
            LifecycleEvent::Unwound {
                checkout,
                failures: slot.teardown.clone(),
            },
        );
        if slot.registered || slot.checkout.is_some() {
            return;
        }
        let Some(state) = slot.pending_end.take() else {
            return;
        };
        let failed_rung = slot.rung.take();
        if let Some(rung) = failed_rung {
            let reason = match &state {
                Lifecycle::Refused(reason) => reason.clone(),
                Lifecycle::NeedsLogin(_) => "the engine needs login".into(),
                _ => "the respawn did not reach a live engine".into(),
            };
            self.report_record(slot, LifecycleEvent::Failed { rung, reason });
        }
        let schedule = state == Lifecycle::Exited
            || (failed_rung.is_some()
                && !slot.install_attempt
                && matches!(state, Lifecycle::Refused(_)));
        slot.state = state;
        if schedule && !slot.cancelled {
            self.schedule(slot, now);
        }
    }

    fn teardown_problem(&mut self, slot: &mut Slot, reason: String) {
        self.problem(&slot.hire.id, "teardown", reason.clone());
        slot.teardown.push(reason);
    }

    fn schedule(&mut self, slot: &mut Slot, now: Instant) {
        if self.stopping || slot.hire.exit_policy == ExitPolicy::Offer {
            return;
        }
        if let Err(error) = self.standing_stop(&slot.hire.id) {
            self.report_record(
                slot,
                LifecycleEvent::Blocked {
                    reason: error.to_string(),
                },
            );
            return;
        }
        if let Some(event) = slot.ladder.schedule(now, slot.hire.exit_policy) {
            if let LifecycleEvent::Exhausted { reason, .. } = &event {
                self.problem(&slot.hire.id, "respawn", reason.clone());
            }
            self.report_record(slot, event);
        }
    }

    fn due_respawn(&mut self, slot: &mut Slot, now: Instant) {
        if slot.reserved() || self.stopping || slot.cancelled {
            return;
        }
        match slot.ladder.due(now) {
            Some(Due::Held(event)) => self.report_record(slot, event),
            Some(Due::Launch(rung)) => {
                if let Err(error) = self.admit(&slot.hire) {
                    self.report_record(
                        slot,
                        LifecycleEvent::Blocked {
                            reason: error.to_string(),
                        },
                    );
                    return;
                }
                let ladder = slot.ladder.clone();
                let session = slot.session_id.clone();
                match self.make_slot(Arc::clone(&slot.hire), ladder, session, Some(rung), true) {
                    Ok(replacement) => *slot = replacement,
                    Err(error) => {
                        let reason = error.to_string();
                        self.report_record(
                            slot,
                            LifecycleEvent::Failed {
                                rung,
                                reason: reason.clone(),
                            },
                        );
                        self.problem(&slot.hire.id, "respawn", reason);
                        self.schedule(slot, now);
                    }
                }
            }
            None => {}
        }
    }

    fn with_slot<T>(
        &mut self,
        agent: &AgentId,
        action: impl FnOnce(&mut Self, &mut Slot) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut slot = self
            .agents
            .remove(agent)
            .ok_or_else(|| Error::Unknown(agent.clone()))?;
        let result = action(self, &mut slot);
        self.agents.insert(agent.clone(), slot);
        result
    }

    fn report_record(&mut self, slot: &Slot, event: LifecycleEvent) {
        if slot.attempt.is_none() {
            return;
        }
        if let Err(error) = self.journal.record(record(slot, event)) {
            self.problem(&slot.hire.id, "log", error.to_string());
        }
    }

    fn clear_problem(&mut self, agent: &AgentId, slug: &str) {
        if let Err(error) = self.journal.clear(cause(agent, slug)) {
            self.events.push(Event::Problem {
                agent: agent.clone(),
                detail: error.to_string(),
            });
        }
    }

    fn problem(&mut self, agent: &AgentId, slug: &str, detail: String) {
        if let Err(error) = self.journal.report(cause(agent, slug), detail.clone()) {
            self.events.push(Event::Problem {
                agent: agent.clone(),
                detail: error.to_string(),
            });
        }
        self.events.push(Event::Problem {
            agent: agent.clone(),
            detail,
        });
    }
}

fn persist_hire(runtime: &Runtime, hire: &Hire, cwd: &Path) -> Result<(), Error> {
    runtime
        .agora
        .edit_registry(|registry| {
            if registry
                .agents
                .get(&hire.id)
                .is_some_and(|agent| agent.retired_at.is_some())
            {
                return Err(StoreError::Invalid(SchemaError::invalid(
                    "registry.json",
                    "agents",
                    format!("{} is retired", hire.id),
                )));
            }
            let mut declared = hire.roster.clone();
            declared.cwd = cwd.display().to_string();
            let _previous = registry.agents.insert(hire.id.clone(), declared);
            Ok(())
        })
        .map_err(|error| Error::Refused(error.to_string()))?;
    runtime.agora.committer().commit_soon("agents: record hire");
    Ok(())
}

fn record(slot: &Slot, event: LifecycleEvent) -> LifecycleRecord {
    LifecycleRecord {
        agent: slot.hire.id.clone(),
        attempt: slot.attempt,
        event,
    }
}

fn installer_cwd(runtime: &Runtime) -> Result<PathBuf, Error> {
    launch::owned_dir(&runtime.home, Path::new("engines"), false)
}

fn running(slot: &mut Slot, generation: Generation) -> Result<&mut Terminal, Error> {
    if slot.generation != generation || slot.state != Lifecycle::Running {
        return Err(Error::Busy(slot.hire.id.clone()));
    }
    slot.terminal
        .as_mut()
        .ok_or_else(|| Error::Busy(slot.hire.id.clone()))
}

fn send_text(slot: &mut Slot, generation: Generation, text: String) -> Result<(), Error> {
    slot.commands.accepted();
    running(slot, generation)?.enqueue(text.into_bytes(), InputKind::Text { arm_submit: true })
}

fn process_result(status: ExitStatus) -> ProcessResult {
    match status {
        ExitStatus::Code(code) => ProcessResult::Code { code },
        ExitStatus::Signal(signal) => ProcessResult::Signal { signal },
    }
}

fn cause(agent: &AgentId, slug: &str) -> Cause {
    Cause {
        source: Source::Agents,
        slug: slug.into(),
        subject: Some(agent.to_string()),
    }
}

fn installer(command: CommandSpec, cwd: &Path) -> InstallCommand {
    InstallCommand {
        program: command.program.into(),
        args: command.args.iter().map(|arg| (*arg).into()).collect(),
        cwd: cwd.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{env, io};

    use eph_engines::Claude;
    use eph_git::committer::Hooks;
    use eph_schema::names::EnvName;

    use super::*;
    use crate::{HookRegistration, fixture_hire, scratch::Scratch};

    struct NoPorts;

    impl Services for NoPorts {
        fn grant(&mut self, _: &AgentId, _: &EnvName) -> Result<Option<String>, Error> {
            Ok(None)
        }

        fn memory(&mut self, _: &AgentId) -> Result<String, Error> {
            Ok(String::new())
        }

        fn register(&mut self, _: &AgentId, _: Generation) -> Result<HookRegistration, Error> {
            Err(Error::Refused("the fixture does not register hooks".into()))
        }

        fn revoke(&mut self, _: &AgentId, _: Generation) -> Result<(), Error> {
            Ok(())
        }
    }

    fn manager(scratch: &Scratch) -> Manager<NoPorts> {
        let prompts = scratch.0.join("prompts");
        fs::create_dir(&prompts).unwrap();
        let agora = Arc::new(
            Agora::open(&scratch.0.join("agora"), &prompts, Hooks::none())
                .unwrap()
                .agora,
        );
        Manager::new(
            &scratch.0,
            agora,
            AdapterRegistry::production(Claude::new(scratch.0.join("unused-hook"))),
            Git::target().without_machine_config(),
            Vec::new(),
            TerminalSize {
                columns: 80,
                rows: 24,
            },
            NoPorts,
        )
        .unwrap()
    }

    fn slot(scratch: &Scratch) -> Slot {
        let id = "agent.fixture".parse().unwrap();
        Slot::new(
            Arc::new(fixture_hire::hire(&id, &scratch.0)),
            Generation(1),
            Ladder::default(),
            None,
            None,
            false,
        )
    }

    fn terminal(manager: &Manager<NoPorts>, scratch: &Scratch) -> Terminal {
        let executable = env::current_exe().unwrap();
        let parent: Environment = env::vars_os().collect();
        let mut environment = base_environment(&parent);
        environment.push(("EPH_AGENTS_UNIT_PTY_HOLD".into(), "1".into()));
        let session = manager
            .open_pty(
                &executable,
                &[
                    "--exact".into(),
                    "manager::tests::pty_fixture_waits_for_owner".into(),
                ],
                scratch.0.parent().unwrap(),
                &environment,
                Vec::new(),
            )
            .unwrap();
        Terminal::new(TerminalKind::Engine, session)
    }

    #[test]
    fn pty_fixture_waits_for_owner() {
        if env::var_os("EPH_AGENTS_UNIT_PTY_HOLD").is_some()
            && env::args().any(|arg| arg == "--exact")
        {
            let mut line = String::new();
            io::stdin().read_line(&mut line).unwrap();
        }
    }

    fn requested(slot: &Slot) -> LifecycleRecord {
        record(
            slot,
            LifecycleEvent::Requested {
                engine: slot.hire.roster.engine,
                workspace: slot.hire.target.display().to_string(),
                grants: Vec::new(),
            },
        )
    }

    #[test]
    fn the_default_transcript_observation_is_unknown() {
        let id = "agent.fixture".parse().unwrap();
        assert_eq!(NoPorts.transcript_present(&id, "session"), None);
    }

    #[test]
    fn input_acknowledgements_preserve_ownership_and_key_timing() {
        let scratch = Scratch::new();
        let mut manager = manager(&scratch);
        let mut slot = slot(&scratch);
        let now = Instant::now();
        slot.state = Lifecycle::Running;
        slot.registered = true;
        slot.terminal = Some(terminal(&manager, &scratch));
        let id = slot.hire.id.clone();
        let _previous = manager.agents.insert(id.clone(), slot);
        assert_eq!(manager.next_deadline(now), Some(now + POLL));
        let mut slot = manager.agents.remove(&id).unwrap();
        let mut terminal = slot.terminal.take().unwrap();
        terminal.input = Some(InputKind::Text { arm_submit: true });
        terminal.disarm_text();
        manager.acknowledge_input(&mut slot, terminal.input.take(), Ok(()), now);
        assert_eq!(slot.commands.deadline(), None);

        slot.state = Lifecycle::Cleaning;
        manager.acknowledge_input(
            &mut slot,
            Some(InputKind::Text { arm_submit: true }),
            Ok(()),
            now,
        );
        assert_eq!(slot.commands.deadline(), None);

        slot.state = Lifecycle::Running;
        manager.acknowledge_input(
            &mut slot,
            Some(InputKind::Text { arm_submit: true }),
            Ok(()),
            now,
        );
        let first = now + Duration::from_millis(150);
        assert_eq!(slot.commands.deadline(), Some(first));
        assert!(matches!(slot.commands.due(first), Some(SubmitKey::Press)));
        let late = first + Duration::from_millis(300);
        manager.acknowledge_input(&mut slot, Some(InputKind::Key), Ok(()), late);
        assert_eq!(
            slot.commands.deadline(),
            Some(late + Duration::from_secs(2))
        );
        manager.acknowledge_input(
            &mut slot,
            Some(InputKind::Key),
            Err(io::Error::other("write failed")),
            late,
        );
        assert_eq!(slot.commands.deadline(), None);
        assert_eq!(
            manager.events,
            [Event::Problem {
                agent: slot.hire.id.clone(),
                detail: "write failed".into()
            }]
        );
        terminal.interrupt = Some(vec![27]);
        manager.poll_input(&mut slot, &mut terminal, late);
        assert!(terminal.interrupt.is_none());
        assert!(matches!(terminal.input, Some(InputKind::Interrupt)));
    }

    #[test]
    fn started_cannot_revive_a_cancelled_or_failed_terminal() {
        let scratch = Scratch::new();
        let mut manager = manager(&scratch);
        let mut slot = slot(&scratch);
        let mut terminal = terminal(&manager, &scratch);
        let now = Instant::now();
        slot.attempt = NonZeroU64::new(manager.journal.acknowledge(requested(&slot)).unwrap());

        slot.state = Lifecycle::Cleaning;
        slot.cancelled = true;
        slot.terminal = Some(terminal);
        manager.on_started(&mut slot, Ok(()), false, now);
        assert_eq!(slot.state, Lifecycle::Cleaning);
        terminal = slot.terminal.take().unwrap();
        slot.cancelled = false;

        for message in [
            "first observation failed",
            "first observation failed",
            "second observation failed",
        ] {
            manager.observe_terminal_exit(&mut slot, &mut terminal, Err(io::Error::other(message)));
        }
        assert_eq!(manager.events.len(), 2);
        assert_eq!(
            terminal.warning.as_deref(),
            Some("terminal exit is unconfirmed: second observation failed")
        );
        assert!(terminal.end.is_none());
        slot.terminal = Some(terminal);
        manager.on_started(&mut slot, Ok(()), false, now);
        assert_eq!(slot.state, Lifecycle::Cleaning);
        terminal = slot.terminal.take().unwrap();
        terminal.warning = None;

        manager.observe_terminal_exit(&mut slot, &mut terminal, Ok(Some(ExitStatus::Code(7))));
        assert_eq!(terminal.end, Some(ProcessResult::Code { code: 7 }));
        slot.terminal = Some(terminal);
        manager.on_started(&mut slot, Ok(()), false, now);
        assert_eq!(slot.state, Lifecycle::Cleaning);

        let mut barrier = requested(&slot);
        barrier.attempt = None;
        let _seq = manager.journal.acknowledge(barrier).unwrap();
        let book = manager.runtime.agora.tail(20).unwrap();
        assert_eq!(
            book.entries
                .iter()
                .filter(|entry| entry.event == "observed")
                .count(),
            2
        );
        manager.runtime.agora.committer().drained();
    }

    #[test]
    fn the_journal_poll_deadline_survives_an_unobserved_failure() {
        let scratch = Scratch::new();
        let mut manager = manager(&scratch);
        let slot = slot(&scratch);
        let now = Instant::now();
        assert_eq!(manager.next_deadline(now), None);
        let _seq = manager.journal.acknowledge(requested(&slot)).unwrap();
        manager.runtime.agora.committer().drained();
        let book = manager.runtime.agora.root().join("log.jsonl");
        let saved = book.with_extension("saved");
        fs::rename(&book, &saved).unwrap();
        fs::create_dir(&book).unwrap();
        manager.journal.record(requested(&slot)).unwrap();
        assert_eq!(manager.next_deadline(now), Some(now + POLL));
        let notice = manager
            .notices
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(notice.agent, slot.hire.id);
        assert!(
            notice
                .detail
                .contains("lifecycle record was not acknowledged")
        );
        manager.journal.notice_received();
        assert_eq!(manager.next_deadline(now), None);
        fs::remove_dir(&book).unwrap();
        fs::rename(saved, book).unwrap();
        assert!(manager.drive(now).is_empty());
    }
}

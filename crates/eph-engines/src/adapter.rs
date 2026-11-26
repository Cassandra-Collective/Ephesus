//! The engine seam and the reference engine's private launch plan (SDD-RUST §3).

use std::{
    collections::{BTreeMap, btree_map::Entry},
    ffi::OsString,
    path::{Path, PathBuf},
};

use eph_fs::home::Condition;
use eph_schema::{
    gates::Autonomy,
    names::{AgentId, EnvName},
    registry::EngineId,
};
use serde_json::Value;
use thiserror::Error;

use crate::{
    BinarySpec, ConfigDir, ConfigError, ResolvedTools, TrustRecord, Workspace,
    config::{CLAUDE_SETTINGS, prepare_claude, refused, trust_claude},
    env,
    prompt::{RenderedPrompt, identity},
    settings::{self, SettingsPlan},
    spec::{CLAUDE, CODEX, GEMINI},
};

/// The environment, with values carried as operating-system strings.
pub type Environment = Vec<(OsString, OsString)>;

/// The engine's declarable sources of lifecycle events (ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSupport {
    /// The engine declares the required native lifecycle events.
    Native,
    /// An engine deliberately synthesizes some events.
    Wrapper,
    /// The adapter supplies no events.
    None,
}

/// Whether a composed autonomy level changes the actual plan (ADR-0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomySupport {
    /// The engine can be configured for every level.
    Enforced,
    /// The adapter cannot enforce the level.
    None,
}

/// An engine-specific notification understood at the adapter boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// The engine is waiting for permission to carry out an action.
    Permission,
    /// The engine is idle and waiting for input.
    Waiting,
}

/// An engine's interrupt bytes and their display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupt {
    /// What the Architect sees.
    pub label: &'static str,
    /// The sequence to write directly to the PTY.
    pub bytes: &'static [u8],
}

/// The company identity applied to Git in the child, when configured (ADR-0022).
#[derive(Debug, Clone, Copy)]
pub struct CommitIdentity<'a> {
    /// The author and committer name.
    pub name: &'a str,
    /// The author and committer address.
    pub email: &'a str,
}

/// A validated, explicitly declared shell permission (ADR-0035).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnattendedGrant {
    run: String,
    prefix: bool,
}

impl UnattendedGrant {
    /// Validate a single command or a prefix of two or more words.
    pub fn new(run: &str, prefix: bool) -> Result<Self, AdapterError> {
        if run.is_empty()
            || run.encode_utf16().count() > 200
            || run.trim() != run
            || run.contains([';', '&', '|', '`', '\n', '\r', '<', '>', '\0'])
            || run.contains("$(")
            || run.contains("..")
            || (prefix && run.split_whitespace().take(2).count() < 2)
        {
            return Err(AdapterError::Unattended);
        }
        Ok(Self {
            run: run.to_owned(),
            prefix,
        })
    }

    pub(crate) fn bash_rule(&self) -> String {
        let suffix = if self.prefix { ":*" } else { "" };
        format!("Bash({}{suffix})", self.run)
    }
}

/// All inputs the manager supplies for one launch. Values do not enter the log.
pub struct SpawnConfig<'a> {
    /// The hire's validated identifier.
    pub agent_id: &'a AgentId,
    /// The per-spawn hook credential.
    pub hook_token: &'a str,
    /// The owned hook endpoint.
    pub hook_endpoint: &'a str,
    /// The approved process working directory.
    pub cwd: &'a Path,
    /// The private configuration for this engine and hire.
    pub engine_config: &'a ConfigDir,
    /// The parent environment from which the base allowlist is selected.
    pub parent_environment: &'a [(OsString, OsString)],
    /// Explicit broker grants with already-validated names.
    pub env_grants: &'a [(EnvName, OsString)],
    /// The company author identity, when configured.
    pub commit_identity: Option<CommitIdentity<'a>>,
    /// The home directory containing prompt overrides.
    pub prompts_dir: &'a Path,
    /// The hire's own identity file.
    pub identity_path: &'a Path,
    /// The Agora's protocol file.
    pub protocol_path: &'a Path,
    /// The caller's already-composed memory.
    pub memory: &'a str,
    /// The recall command, when available.
    pub recall_command: Option<&'a str>,
    /// The narrowly scoped GitHub credential refresh command, when available.
    pub credential_refresh: Option<&'a str>,
    /// A declared directory of runbooks to grant read access to.
    pub playbooks_dir: Option<&'a Path>,
    /// Tool directories already resolved against the named roots.
    pub tools: &'a ResolvedTools,
    /// Exact commands and prefixes declared before activation.
    pub unattended: &'a [UnattendedGrant],
    /// The watch's composed level (FR-1.10).
    pub autonomy: Autonomy,
}

/// Literal executable, arguments and environment plus the settings to install.
pub struct SpawnPlan {
    program: &'static str,
    args: Vec<OsString>,
    cwd: PathBuf,
    environment: Environment,
    settings: Option<SettingsPlan>,
    prompt_condition: Option<Condition>,
}

impl SpawnPlan {
    /// The executable name, resolved by the terminal from the plan's PATH.
    pub fn program(&self) -> &'static str {
        self.program
    }

    /// Literal arguments after the executable.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// The approved process working directory.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// The exact environment accepted by the terminal plane.
    pub fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    /// The private settings file this launch needs, when it declares one.
    pub fn settings(&self) -> Option<&SettingsPlan> {
        self.settings.as_ref()
    }

    /// An unreadable home override reported before using the bundled prompt.
    pub fn prompt_condition(&self) -> Option<&Condition> {
        self.prompt_condition.as_ref()
    }
}

/// A fact an optional transcript reader can return (R6, ADR-0009).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageFact {
    /// The engine's session id.
    pub session: String,
    /// The engine's reported model.
    pub model: String,
    /// The input tokens stated by the engine, including charged cache use.
    pub input_tokens: u64,
    /// The output tokens stated by the engine.
    pub output_tokens: u64,
}

/// Reads an engine's external transcript format when that capability exists.
pub trait TranscriptReader {
    /// Read facts from the named transcript. The reader arrives with R6.
    fn read(&self, file: &Path) -> Result<Vec<UsageFact>, std::io::Error>;
}

/// The adapter contract. The manager and the terminal consume only this surface.
pub trait EngineAdapter: Send + Sync {
    /// Installed binary, install offer and probe declarations.
    fn binary(&self) -> BinarySpec;
    /// The lifecycle events this adapter's settings can supply.
    fn hooks(&self) -> HookSupport;
    /// Whether the adapter can enforce the request's composed autonomy.
    fn autonomy(&self) -> AutonomySupport;
    /// Render identity, settings, and literal child arguments.
    fn plan(&self, config: &SpawnConfig<'_>) -> Result<SpawnPlan, AdapterError>;
    /// Bytes to interrupt a running agent.
    fn interrupt(&self) -> Interrupt {
        Interrupt {
            label: "Escape",
            bytes: b"\x1b",
        }
    }
    /// Literal arguments for reopening a session, where supported.
    fn resume(&self, _session: &str) -> Option<Vec<OsString>> {
        None
    }
    /// Prepare the private config and return the new record, if created or updated.
    fn prepare(&self, _config: &ConfigDir) -> Result<Option<PathBuf>, ConfigError> {
        Ok(None)
    }
    /// Record an approved workspace when the engine supports pre-granting trust.
    fn trust(
        &self,
        _config: &ConfigDir,
        _workspace: Workspace<'_>,
    ) -> Option<Result<TrustRecord, ConfigError>> {
        None
    }
    /// The private authentication environment before grants or hook registration.
    fn authentication_environment(
        &self,
        _agent: &AgentId,
        _config: &ConfigDir,
        _parent: &[(OsString, OsString)],
    ) -> Result<Option<Environment>, ConfigError> {
        Ok(None)
    }
    /// A transcript reader when the adapter declares one.
    fn transcripts(&self) -> Option<&dyn TranscriptReader> {
        None
    }
    /// Interpret a notification where the adapter can establish its meaning.
    fn notification(&self, _payload: &Value) -> Option<NotificationKind> {
        None
    }
}

/// An adapter or its requested capabilities could not be used.
#[derive(Debug, Error)]
pub enum AdapterError {
    /// An engine-specific path or configuration cannot be used.
    #[error(transparent)]
    Configuration(#[from] ConfigError),
    /// A registration already exists for this engine id.
    #[error("engine {0:?} was registered twice; register one adapter per engine")]
    Duplicate(EngineId),
    /// The MVP refuses any engine other than Claude at profile and hire boundaries.
    #[error("engine {0:?} cannot be hired: the MVP ships Claude Code only (ADR-0024)")]
    NonReference(EngineId),
    /// A plan cannot honor the composed autonomy ceiling.
    #[error(
        "engine {engine:?} cannot enforce {level:?}; choose an engine that enforces the requested autonomy (FR-1.10)"
    )]
    Unenforceable {
        /// The adapter the hire selected.
        engine: EngineId,
        /// The level that would be ignored.
        level: Autonomy,
    },
    /// A grant would be wider than the declared one-command vocabulary.
    #[error(
        "an unattended grant needs one trimmed command of 1–200 characters without chaining or traversal; a prefix needs two words (ADR-0035)"
    )]
    Unattended,
}

/// Refuse a profile or hire outside the shipping engine (ADR-0024).
pub fn require_reference_engine(engine: EngineId) -> Result<(), AdapterError> {
    if engine == EngineId::Claude {
        Ok(())
    } else {
        Err(AdapterError::NonReference(engine))
    }
}

/// Apply the engine's autonomy grade before creating a process (FR-1.10).
pub fn assert_autonomy(adapter: &dyn EngineAdapter, level: Autonomy) -> Result<(), AdapterError> {
    if level != Autonomy::Autonomous && adapter.autonomy() == AutonomySupport::None {
        return Err(AdapterError::Unenforceable {
            engine: adapter.binary().engine,
            level,
        });
    }
    Ok(())
}

/// One explicit registration per engine id (ADR-0009).
#[derive(Default)]
pub struct AdapterRegistry {
    adapters: BTreeMap<EngineId, Box<dyn EngineAdapter>>,
}

impl AdapterRegistry {
    /// Production ships only the provided reference adapter.
    pub fn production(claude: Claude) -> Self {
        Self {
            adapters: BTreeMap::from([(
                EngineId::Claude,
                Box::new(claude) as Box<dyn EngineAdapter>,
            )]),
        }
    }

    /// Register an adapter once. Tests can install nonshipping conformance subjects.
    pub fn register(&mut self, adapter: Box<dyn EngineAdapter>) -> Result<(), AdapterError> {
        let id = adapter.binary().engine;
        match self.adapters.entry(id) {
            Entry::Vacant(slot) => {
                slot.insert(adapter);
                Ok(())
            }
            Entry::Occupied(_) => Err(AdapterError::Duplicate(id)),
        }
    }

    /// Find a registered engine by id.
    pub fn get(&self, engine: EngineId) -> Option<&dyn EngineAdapter> {
        self.adapters.get(&engine).map(Box::as_ref)
    }

    /// The registered ids in vocabulary order.
    pub fn engines(&self) -> impl Iterator<Item = EngineId> + '_ {
        self.adapters.keys().copied()
    }
}

/// The reference adapter. The hook executable is supplied by its caller (R3).
pub struct Claude {
    hook: PathBuf,
    usage: Option<(PathBuf, PathBuf)>,
}

impl Claude {
    /// Supply the absolute path of the harness's hook bridge.
    pub fn new(hook: PathBuf) -> Self {
        Self { hook, usage: None }
    }

    /// Supply the usage shim and its report directory together (ADR-0023).
    pub fn with_usage(mut self, executable: PathBuf, reports: PathBuf) -> Self {
        self.usage = Some((executable, reports));
        self
    }
}

impl EngineAdapter for Claude {
    fn binary(&self) -> BinarySpec {
        CLAUDE
    }

    fn hooks(&self) -> HookSupport {
        HookSupport::Native
    }

    fn autonomy(&self) -> AutonomySupport {
        AutonomySupport::Enforced
    }

    fn plan(&self, config: &SpawnConfig<'_>) -> Result<SpawnPlan, AdapterError> {
        let settings = settings::render(config, &self.hook, self.usage.as_ref())?;
        let mut environment = env::spawn(config);
        env::claude(
            config.engine_config,
            config.parent_environment,
            &mut environment,
        );
        build_plan(
            config,
            CLAUDE,
            claude_arguments(config),
            environment,
            Some(settings),
        )
    }

    fn resume(&self, session: &str) -> Option<Vec<OsString>> {
        Some(vec!["--resume".into(), session.into()])
    }

    fn prepare(&self, config: &ConfigDir) -> Result<Option<PathBuf>, ConfigError> {
        prepare_claude(config)
    }

    fn trust(
        &self,
        config: &ConfigDir,
        workspace: Workspace<'_>,
    ) -> Option<Result<TrustRecord, ConfigError>> {
        Some(trust_claude(config, workspace))
    }

    fn authentication_environment(
        &self,
        agent: &AgentId,
        config: &ConfigDir,
        parent: &[(OsString, OsString)],
    ) -> Result<Option<Environment>, ConfigError> {
        config.require_engine(EngineId::Claude)?;
        config.require_agent(agent)?;
        let mut environment = env::base_environment(parent);
        env::claude(config, parent, &mut environment);
        Ok(Some(environment))
    }
}

/// The two declared conformance subjects kept out of production (ADR-0024).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceAdapter {
    /// Codex's working directory argument and positional identity.
    Codex,
    /// Gemini's interactive positional identity.
    Gemini,
}

impl EngineAdapter for ConformanceAdapter {
    fn binary(&self) -> BinarySpec {
        match self {
            Self::Codex => CODEX,
            Self::Gemini => GEMINI,
        }
    }

    fn hooks(&self) -> HookSupport {
        HookSupport::None
    }

    fn autonomy(&self) -> AutonomySupport {
        AutonomySupport::None
    }

    fn plan(&self, config: &SpawnConfig<'_>) -> Result<SpawnPlan, AdapterError> {
        build_plan(
            config,
            self.binary(),
            self.conformance_arguments(config),
            env::spawn(config),
            None,
        )
    }
}

impl ConformanceAdapter {
    fn conformance_arguments(self, config: &SpawnConfig<'_>) -> Vec<OsString> {
        match self {
            Self::Codex => vec!["--cd".into(), config.cwd.as_os_str().to_owned()],
            Self::Gemini => Vec::new(),
        }
    }
}

fn build_plan(
    config: &SpawnConfig<'_>,
    binary: BinarySpec,
    mut args: Vec<OsString>,
    environment: Environment,
    settings: Option<SettingsPlan>,
) -> Result<SpawnPlan, AdapterError> {
    config.engine_config.require_engine(binary.engine)?;
    config.engine_config.require_agent(config.agent_id)?;
    if !config.cwd.is_absolute() {
        return Err(refused(
            config.cwd,
            "the spawn requires an approved absolute directory",
        )
        .into());
    }
    let (prompt, prompt_condition) = identity(config)?;
    send_identity(prompt, &mut args);
    Ok(SpawnPlan {
        program: binary.version.program,
        args,
        cwd: config.cwd.to_path_buf(),
        environment,
        settings,
        prompt_condition,
    })
}

fn claude_arguments(config: &SpawnConfig<'_>) -> Vec<OsString> {
    let permission_mode = match config.autonomy {
        Autonomy::Manual => "default",
        Autonomy::Supervised => "acceptEdits",
        Autonomy::Autonomous => "auto",
    };
    let mut args = vec![
        "--permission-mode".into(),
        permission_mode.into(),
        "--setting-sources=".into(),
        "--settings".into(),
        config
            .engine_config
            .path()
            .join(CLAUDE_SETTINGS)
            .into_os_string(),
    ];
    for directory in config.tools.directories() {
        args.extend(["--plugin-dir".into(), directory.as_os_str().to_owned()]);
    }
    args.push("--append-system-prompt".into());
    args
}

fn send_identity(prompt: RenderedPrompt, args: &mut Vec<OsString>) {
    prompt.append_argument(args);
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use eph_fs::atomic::write_atomic;
    use eph_pty::{ExitStatus, Output, Session, SpawnPlan as TerminalPlan};

    use super::*;
    use crate::{spec::FAKE, test_support::script};

    struct Fake;

    impl EngineAdapter for Fake {
        fn binary(&self) -> BinarySpec {
            FAKE
        }

        fn hooks(&self) -> HookSupport {
            HookSupport::None
        }

        fn autonomy(&self) -> AutonomySupport {
            AutonomySupport::None
        }

        fn plan(&self, config: &SpawnConfig<'_>) -> Result<SpawnPlan, AdapterError> {
            build_plan(config, FAKE, Vec::new(), env::spawn(config), None)
        }
    }

    #[test]
    fn the_fake_subject_drives_the_adapter_plan_through_the_owned_terminal() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("target");
        let prompts = root.path().join("prompts");
        fs::create_dir_all(prompts.join("engines")).unwrap();
        fs::create_dir(&cwd).unwrap();
        let identity = root.path().join("identity.md");
        write_atomic(&identity, b"fixture prompt").unwrap();
        write_atomic(
            &prompts.join("engines/identity-appendix.md"),
            b"{{identity}}",
        )
        .unwrap();
        script(
            root.path(),
            "eph-fake-engine",
            "@echo off\r\necho ready:%~1\r\nset /p input=\r\necho typed:%input%\r\nexit /b 7\r\n",
            "#!/bin/sh\nprintf 'ready:%s\\n' \"$1\"\nIFS= read -r input\nprintf 'typed:%s\\n' \"$input\"\nexit 7\n",
        );
        let fake = Fake;
        let claude = Claude::new(root.path().join("eph-hook"));
        let subjects: [&dyn EngineAdapter; 4] = [
            &fake,
            &claude,
            &ConformanceAdapter::Codex,
            &ConformanceAdapter::Gemini,
        ];
        assert_eq!(
            subjects.map(|adapter| adapter.binary().engine()),
            [
                EngineId::Custom,
                EngineId::Claude,
                EngineId::Codex,
                EngineId::Gemini
            ]
        );
        let id = "agent.fake".parse().unwrap();
        let private = ConfigDir::for_agent(root.path(), EngineId::Custom, &id).unwrap();
        let mut parent = env::base_environment(&std::env::vars_os().collect::<Vec<_>>());
        env::put(&mut parent, "PATH", root.path());
        let config = SpawnConfig {
            agent_id: &id,
            hook_token: "fixture-token",
            hook_endpoint: "fixture-endpoint",
            cwd: &cwd,
            engine_config: &private,
            parent_environment: &parent,
            env_grants: &[],
            commit_identity: None,
            prompts_dir: &prompts,
            identity_path: &identity,
            protocol_path: &identity,
            memory: "",
            recall_command: None,
            credential_refresh: None,
            playbooks_dir: None,
            tools: &ResolvedTools::default(),
            unattended: &[],
            autonomy: Autonomy::Autonomous,
        };
        let plan = fake.plan(&config).unwrap();
        assert_eq!(fake.hooks(), HookSupport::None);
        assert!(assert_autonomy(&fake, config.autonomy).is_ok());
        assert!(plan.settings().is_none());
        let mut session = Session::spawn(
            TerminalPlan {
                executable: plan.program().as_ref(),
                args: plan.args(),
                cwd: plan.cwd(),
                environment: plan.environment(),
                cols: 80,
                rows: 24,
            },
            Vec::new(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stdout = Vec::new();
        while !String::from_utf8_lossy(&stdout).contains("ready:fixture prompt") {
            assert!(
                Instant::now() < deadline,
                "fixture did not receive the plan"
            );
            match session.output(Duration::from_millis(20)).unwrap() {
                Output::Data(bytes) => stdout.extend(bytes),
                Output::Pending => {}
                Output::End => panic!("fixture ended before reading input"),
            }
        }
        session.write(b"from-pty\r").unwrap();
        let mut exit = None;
        loop {
            assert!(Instant::now() < deadline, "fixture did not finish");
            if exit.is_none() {
                exit = session.try_wait().unwrap();
            }
            match session.output(Duration::from_millis(20)).unwrap() {
                Output::Data(bytes) => stdout.extend(bytes),
                Output::Pending => {}
                Output::End if exit.is_some() => break,
                Output::End => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        assert_eq!(exit, Some(ExitStatus::Code(7)));
        assert!(String::from_utf8_lossy(&stdout).contains("typed:from-pty"));
    }
}

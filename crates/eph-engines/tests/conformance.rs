//! Replay the engine-side conformance contract without installed CLI binaries.

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

use eph_engines::{
    AdapterRegistry, Authentication, AutonomySupport, Claude, CommitIdentity, ConfigDir,
    ConformanceAdapter, EngineAdapter, HookSupport, ResolvedTools, SpawnConfig, ToolGrant,
    ToolRoot, ToolRoots, UnattendedGrant, Workspace, assert_autonomy, require_reference_engine,
    resolve_tool_grants,
};
use eph_fs::atomic::write_atomic;
use eph_schema::{gates::Autonomy, names::AgentId, registry::EngineId};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

struct Rig {
    root: TempDir,
    cwd: PathBuf,
    identity: PathBuf,
    protocol: PathBuf,
    prompts: PathBuf,
    home: PathBuf,
    id: AgentId,
    config: ConfigDir,
    env: Vec<(OsString, OsString)>,
    tools: ResolvedTools,
}

impl Rig {
    #[expect(
        clippy::unwrap_used,
        reason = "test setup must fail when its temporary files cannot be prepared"
    )]
    fn new(engine: EngineId) -> Self {
        let root = tempdir().unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("repo");
        let agent = root.path().join("agent");
        let prompts = home.join("prompts");
        let identity = agent.join("identity.md");
        let protocol = agent.join("PROTOCOL.md");
        for dir in [&home, &cwd, &agent, &prompts] {
            fs::create_dir_all(dir).unwrap();
        }
        write_atomic(&identity, b"# The hire").unwrap();
        write_atomic(&protocol, b"# The protocol").unwrap();
        write_atomic(&cwd.join("existing"), b"target bytes").unwrap();
        let id = "agent.contract".parse().unwrap();
        let config = ConfigDir::for_agent(&home, engine, &id).unwrap();
        Self {
            root,
            home,
            cwd,
            identity,
            protocol,
            prompts,
            id,
            config,
            env: vec![
                ("Path".into(), "approved path".into()),
                ("CLAUDE_CONFIG_DIR".into(), "operator account".into()),
                ("NOT_GRANTED".into(), "private".into()),
            ],
            tools: ResolvedTools::default(),
        }
    }

    fn config(&self, autonomy: Autonomy) -> SpawnConfig<'_> {
        SpawnConfig {
            agent_id: &self.id,
            hook_token: "test-token",
            hook_endpoint: "test-endpoint",
            cwd: &self.cwd,
            engine_config: &self.config,
            parent_environment: &self.env,
            env_grants: &[],
            commit_identity: None,
            prompts_dir: &self.prompts,
            identity_path: &self.identity,
            protocol_path: &self.protocol,
            memory: "Remember {{identity}} as data.",
            recall_command: Some("eph-recall"),
            credential_refresh: None,
            playbooks_dir: None,
            tools: &self.tools,
            unattended: &[],
            autonomy,
        }
    }

    fn claude(&self) -> Claude {
        Claude::new(self.root.path().join("eph hook"))
    }
}

fn env<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    environment
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_os_str())
}

#[test]
fn each_subject_answers_the_same_adapter_contract_in_both_directions() {
    let root = Rig::new(EngineId::Claude);
    let claude = root.claude();
    let subjects: [&dyn EngineAdapter; 3] = [
        &claude,
        &ConformanceAdapter::Codex,
        &ConformanceAdapter::Gemini,
    ];
    assert_eq!(
        subjects.map(|adapter| adapter.binary().engine()),
        [EngineId::Claude, EngineId::Codex, EngineId::Gemini],
    );
    for adapter in subjects {
        let binary = adapter.binary();
        let rig = Rig::new(binary.engine());
        let low = rig.config(Autonomy::Manual);
        assert_eq!(
            adapter.authentication_environment(&low).unwrap().is_some(),
            binary.authentication().is_some()
        );
        let mut high = rig.config(Autonomy::Autonomous);
        let grants = [("ALLOWED".parse().unwrap(), OsString::from("granted"))];
        high.env_grants = &grants;
        high.commit_identity = Some(CommitIdentity {
            name: "Company",
            email: "company@example.test",
        });
        let high_plan = adapter.plan(&high).unwrap();
        assert_eq!(adapter.interrupt().label, "Escape");
        assert_eq!(adapter.interrupt().bytes, b"\x1b");
        assert_eq!(high_plan.program(), binary.version().program);
        assert_eq!(high_plan.cwd(), rig.cwd);
        assert_eq!(
            env(high_plan.environment(), "Path"),
            Some(OsStr::new("approved path"))
        );
        assert_eq!(
            env(high_plan.environment(), "EPH_HOOK_TOKEN"),
            Some(OsStr::new("test-token"))
        );
        assert_eq!(
            env(high_plan.environment(), "EPH_HOOK_ENDPOINT"),
            Some(OsStr::new("test-endpoint"))
        );
        assert_eq!(
            env(high_plan.environment(), "EPH_AGENT_ID"),
            Some(OsStr::new("agent.contract"))
        );
        assert_eq!(
            env(high_plan.environment(), "EPH_RECALL"),
            Some(OsStr::new("eph-recall"))
        );
        assert_eq!(
            env(high_plan.environment(), "ALLOWED"),
            Some(OsStr::new("granted"))
        );
        assert_eq!(
            env(high_plan.environment(), "GIT_AUTHOR_NAME"),
            Some(OsStr::new("Company"))
        );
        assert_eq!(
            env(high_plan.environment(), "GIT_COMMITTER_EMAIL"),
            Some(OsStr::new("company@example.test"))
        );
        assert_eq!(env(high_plan.environment(), "NOT_GRANTED"), None);
        let prompt = high_plan.args().last().unwrap().to_str().unwrap();
        assert!(prompt.contains("# The hire"));
        assert!(prompt.contains("# The protocol"));
        assert!(prompt.contains("Remember {{identity}} as data."));
        let low_plan = adapter.plan(&low).unwrap();
        let middle_plan = adapter.plan(&rig.config(Autonomy::Supervised)).unwrap();
        match binary.engine() {
            EngineId::Claude => {
                assert_eq!(&low_plan.args()[..2], ["--permission-mode", "default"]);
                assert_eq!(
                    &middle_plan.args()[..2],
                    ["--permission-mode", "acceptEdits"]
                );
                assert_eq!(&high_plan.args()[..2], ["--permission-mode", "auto"]);
            }
            EngineId::Codex => {
                assert_eq!(high_plan.args().len(), 3);
                assert_eq!(high_plan.args()[0], "--cd");
                assert_eq!(high_plan.args()[1], rig.cwd);
            }
            EngineId::Gemini => assert_eq!(high_plan.args().len(), 1),
            other => panic!("unreviewed conformance subject {other:?}"),
        }
        if adapter.autonomy() == AutonomySupport::Enforced {
            assert_ne!(low_plan.args(), middle_plan.args());
            assert_ne!(middle_plan.args(), high_plan.args());
            assert_ne!(low_plan.args(), high_plan.args());
            assert!(assert_autonomy(adapter, Autonomy::Manual).is_ok());
            assert!(assert_autonomy(adapter, Autonomy::Supervised).is_ok());
        } else {
            let matching_high = adapter.plan(&rig.config(Autonomy::Autonomous)).unwrap();
            assert_eq!(low_plan.args(), middle_plan.args());
            assert_eq!(low_plan.args(), matching_high.args());
            assert_eq!(low_plan.environment(), matching_high.environment());
            assert!(assert_autonomy(adapter, Autonomy::Manual).is_err());
            assert!(assert_autonomy(adapter, Autonomy::Supervised).is_err());
        }
        assert!(assert_autonomy(adapter, Autonomy::Autonomous).is_ok());
        let before = fs::read_dir(&rig.cwd).unwrap().count();
        match (adapter.hooks(), high_plan.settings()) {
            (HookSupport::Native, Some(settings)) => {
                let hooks = settings.content()["hooks"].as_object().unwrap();
                assert_eq!(hooks.len(), 9);
                let hook = root.root.path().join("eph hook");
                let quoted_hook = format!("\"{}\" --event ", hook.display());
                for (event, name) in [
                    ("SessionStart", "session-start"),
                    ("UserPromptSubmit", "prompt-submitted"),
                    ("PreToolUse", "pre-tool"),
                    ("PostToolUse", "post-tool"),
                    ("Notification", "notification"),
                    ("Stop", "stop"),
                    ("PreCompact", "compact-start"),
                    ("PostCompact", "compact-end"),
                    ("SessionEnd", "session-end"),
                ] {
                    let command = hooks[event][0]["hooks"][0]["command"].as_str().unwrap();
                    assert!(command.starts_with(&quoted_hook), "{command:?}");
                    assert!(command.contains(&format!(" --event {name} --")));
                    assert!(command.ends_with("--session-field session_id"));
                }
                assert_eq!(settings.path().parent(), Some(rig.config.path()));
                settings.install().unwrap();
                settings.uninstall();
                settings.uninstall();
                assert!(settings.path().is_file());
                assert_eq!(
                    adapter.resume("a").unwrap(),
                    [OsString::from("--resume"), OsString::from("a")]
                );
                assert_ne!(adapter.resume("a"), adapter.resume("b"));
            }
            (HookSupport::None, None) => {
                assert!(!rig.config.path().exists());
                assert!(adapter.resume("a").is_none());
                assert!(adapter.prepare(&rig.config).unwrap().is_none());
                assert!(
                    adapter
                        .trust(&rig.config, Workspace::Existing(&rig.cwd))
                        .is_none()
                );
            }
            _ => panic!("hook grade and actual declarations disagree"),
        }
        assert_eq!(fs::read_dir(&rig.cwd).unwrap().count(), before);
        assert_eq!(fs::read(rig.cwd.join("existing")).unwrap(), b"target bytes");
        assert!(adapter.transcripts().is_none());
        assert!(adapter.notification(&json!({})).is_none());
        fs::remove_file(&rig.identity).unwrap();
        assert!(
            adapter
                .plan(&low)
                .err()
                .unwrap()
                .to_string()
                .contains("identity")
        );
    }
}

#[test]
fn fixture_replay_observes_the_pinned_cli_grammars() {
    let rig = Rig::new(EngineId::Claude);
    let claude = rig.claude().binary();
    for (binary, stdout, version) in [
        (
            claude,
            include_bytes!("fixtures/engine-output/claude/version.txt").as_slice(),
            "2.1.295",
        ),
        (
            ConformanceAdapter::Codex.binary(),
            include_bytes!("fixtures/engine-output/codex/version.txt").as_slice(),
            "0.171.0-alpha.13",
        ),
        (
            ConformanceAdapter::Gemini.binary(),
            include_bytes!("fixtures/engine-output/gemini/version.txt").as_slice(),
            "0.57.0",
        ),
        (
            claude,
            include_bytes!("../../../test/fixtures/engine-output/claude/version.txt").as_slice(),
            "2.1.252",
        ),
    ] {
        assert_eq!(binary.parse_version(stdout), Some(version));
        assert_eq!(binary.version().args, ["--version"]);
        assert_eq!(binary.install().args[..2], ["install", "-g"]);
        assert!(binary.parse_version(b"no output banner").is_none());
        assert!(binary.parse_version(b"\xff").is_none());
    }
    for invalid in [
        b"0..0".as_slice(),
        b"0.x.0",
        b"0.57.0.1",
        b"0.57.0-",
        b"0.57.0-!",
    ] {
        assert!(
            ConformanceAdapter::Gemini
                .binary()
                .parse_version(invalid)
                .is_none()
        );
    }
    let auth = claude.authentication().unwrap();
    assert_eq!(auth.probe().args, ["auth", "status"]);
    assert_eq!(auth.login().args, ["auth", "login"]);
    for bytes in [
        include_bytes!("fixtures/engine-output/claude/auth-status-logged-out.json").as_slice(),
        include_bytes!("fixtures/engine-output/claude/auth-status-isolated-logged-out.txt"),
    ] {
        assert_eq!(auth.classify(bytes), Authentication::NeedsLogin);
    }
    for bytes in [
        include_bytes!("../../../test/fixtures/engine-output/claude/auth-status.json").as_slice(),
        include_bytes!("../../../test/fixtures/engine-output/claude/auth-status-text.txt"),
    ] {
        assert_eq!(auth.classify(bytes), Authentication::Authenticated);
    }
    for bytes in [
        b"".as_slice(),
        b"\xff",
        br#"{"loggedIn":"true"}"#,
        b"unknown auth switch",
    ] {
        assert_eq!(auth.classify(bytes), Authentication::Inconclusive);
    }
}

#[test]
fn the_reference_registry_and_the_hire_refusal_agree() {
    let rig = Rig::new(EngineId::Claude);
    let mut registry = AdapterRegistry::production(rig.claude());
    assert_eq!(registry.engines().collect::<Vec<_>>(), [EngineId::Claude]);
    assert!(registry.get(EngineId::Claude).is_some());
    assert!(require_reference_engine(EngineId::Claude).is_ok());
    assert!(registry.register(Box::new(rig.claude())).is_err());
    for id in [
        EngineId::Codex,
        EngineId::Gemini,
        EngineId::Grok,
        EngineId::Opencode,
        EngineId::Custom,
    ] {
        assert!(registry.get(id).is_none());
        assert!(
            require_reference_engine(id)
                .unwrap_err()
                .to_string()
                .contains("ADR-0024")
        );
    }
    registry
        .register(Box::new(ConformanceAdapter::Codex))
        .unwrap();
    assert_eq!(
        registry.get(EngineId::Codex).unwrap().binary().engine(),
        EngineId::Codex
    );
}

#[test]
fn claude_spawn_and_probe_use_one_private_config_and_credentials_lookup() {
    let mut rig = Rig::new(EngineId::Claude);
    let claude = rig.claude();
    let config = rig.config(Autonomy::Autonomous);
    let plan = claude.plan(&config).unwrap();
    let argv = plan.args();
    assert_eq!(
        &argv[..4],
        [
            "--permission-mode",
            "auto",
            "--setting-sources=",
            "--settings"
        ]
    );
    assert_eq!(argv[4], rig.config.path().join("eph-settings.json"));
    assert_eq!(argv[5], "--append-system-prompt");
    let auth = claude.authentication_environment(&config).unwrap().unwrap();
    assert_eq!(plan.environment(), auth);
    assert_eq!(
        env(&auth, "CLAUDE_CONFIG_DIR"),
        Some(rig.config.path().as_os_str())
    );
    assert_eq!(
        env(&auth, "CLAUDE_SECURESTORAGE_CONFIG_DIR"),
        Some(OsStr::new("operator account"))
    );
    assert_eq!(env(&auth, "DISABLE_AUTOUPDATER"), Some(OsStr::new("1")));
    rig.env.retain(|(key, _)| key != "CLAUDE_CONFIG_DIR");
    rig.env
        .push(("USERPROFILE".into(), rig.home.as_os_str().to_owned()));
    let auth = claude
        .authentication_environment(&rig.config(Autonomy::Manual))
        .unwrap()
        .unwrap();
    assert_eq!(
        env(&auth, "CLAUDE_SECURESTORAGE_CONFIG_DIR"),
        Some(rig.home.join(".claude").as_os_str())
    );
    rig.env.clear();
    let auth = claude
        .authentication_environment(&rig.config(Autonomy::Manual))
        .unwrap()
        .unwrap();
    assert_eq!(
        env(&auth, "CLAUDE_SECURESTORAGE_CONFIG_DIR"),
        Some(OsStr::new(""))
    );
}

#[test]
fn the_config_uses_one_absolute_path_for_the_harness_and_the_child() {
    let rig = Rig::new(EngineId::Claude);
    let derived =
        ConfigDir::for_agent(Path::new("relative-home"), EngineId::Claude, &rig.id).unwrap();
    assert!(derived.path().is_absolute());
    assert!(
        derived
            .path()
            .ends_with("relative-home/engines/claude/agent.contract")
    );
    let mut config = rig.config(Autonomy::Manual);
    let other_id: AgentId = "agent.other".parse().unwrap();
    let other = ConfigDir::for_agent(&rig.home, EngineId::Claude, &other_id).unwrap();
    let wrong_engine = ConfigDir::for_agent(&rig.home, EngineId::Codex, &rig.id).unwrap();
    for replacement in [&other, &wrong_engine] {
        config.engine_config = replacement;
        assert!(rig.claude().plan(&config).is_err());
        assert!(rig.claude().authentication_environment(&config).is_err());
    }
    assert!(rig.claude().prepare(&wrong_engine).is_err());
    assert!(
        rig.claude()
            .trust(&wrong_engine, Workspace::Existing(&rig.cwd))
            .unwrap()
            .is_err()
    );
}

#[test]
fn broker_grants_cannot_override_the_claude_engine_or_optional_helpers() {
    let rig = Rig::new(EngineId::Claude);
    let grants = [
        ("DISABLE_AUTOUPDATER".parse().unwrap(), "0".into()),
        ("CLAUDE_CONFIG_DIR".parse().unwrap(), "other".into()),
        (
            "CLAUDE_SECURESTORAGE_CONFIG_DIR".parse().unwrap(),
            "other".into(),
        ),
        ("EPH_RECALL".parse().unwrap(), "other".into()),
        ("EPH_GH_TOKEN".parse().unwrap(), "other".into()),
    ];
    let mut config = rig.config(Autonomy::Supervised);
    config.recall_command = None;
    config.env_grants = &grants;
    let built = rig.claude().plan(&config).unwrap();
    let actual = built.environment();
    assert_eq!(env(actual, "DISABLE_AUTOUPDATER"), Some(OsStr::new("1")));
    assert_eq!(
        env(actual, "CLAUDE_CONFIG_DIR"),
        Some(rig.config.path().as_os_str())
    );
    assert_eq!(
        env(actual, "CLAUDE_SECURESTORAGE_CONFIG_DIR"),
        Some(OsStr::new("operator account"))
    );
    assert_eq!(env(actual, "EPH_RECALL"), None);
    assert_eq!(env(actual, "EPH_GH_TOKEN"), None);
    config.recall_command = Some("eph-recall");
    config.credential_refresh = Some("eph-gh-token");
    let built = rig.claude().plan(&config).unwrap();
    assert_eq!(
        env(built.environment(), "EPH_RECALL"),
        Some(OsStr::new("eph-recall"))
    );
    assert_eq!(
        env(built.environment(), "EPH_GH_TOKEN"),
        Some(OsStr::new("eph-gh-token"))
    );
}

#[test]
fn onboarding_and_approved_trust_preserve_unrelated_engine_fields() {
    let rig = Rig::new(EngineId::Claude);
    let claude = rig.claude();
    let file = claude.prepare(&rig.config).unwrap().unwrap();
    write_atomic(&file, br#"{"custom":true,"projects":{"kept":{"other":1}}}"#).unwrap();
    assert_eq!(claude.prepare(&rig.config).unwrap().unwrap(), file);
    assert_eq!(claude.prepare(&rig.config).unwrap(), None);
    let worktree = rig.cwd.join("future-worktree");
    let earlier = claude
        .trust(&rig.config, Workspace::FutureWorktree(&worktree))
        .unwrap()
        .unwrap();
    assert!(!earlier.already_trusted);
    assert!(earlier.project.ends_with("/repo/future-worktree"));
    assert!(!worktree.exists());
    fs::create_dir(&worktree).unwrap();
    let later = claude
        .trust(&rig.config, Workspace::Existing(&worktree))
        .unwrap()
        .unwrap();
    assert_eq!(earlier.project, later.project);
    assert!(later.already_trusted);
    let state: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(state["custom"], true);
    assert_eq!(state["hasCompletedOnboarding"], true);
    assert_eq!(state["projects"]["kept"]["other"], 1);
    assert_eq!(
        state["projects"][later.project]["hasTrustDialogAccepted"],
        true
    );
    let missing = rig.cwd.join("absent");
    assert!(
        claude
            .trust(&rig.config, Workspace::Existing(&missing))
            .unwrap()
            .is_err()
    );
    write_atomic(&file, b"[]").unwrap();
    assert!(claude.prepare(&rig.config).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"[]");
}

#[test]
fn regenerated_settings_include_only_current_declared_grants_and_hooks() {
    let rig = Rig::new(EngineId::Claude);
    let usage_path = rig.root.path().join("usage");
    let reports = rig.root.path().join("reports");
    let claude = rig.claude().with_usage(usage_path, reports);
    let old = [UnattendedGrant::new("cargo check", true).unwrap()];
    let mut config = rig.config(Autonomy::Autonomous);
    config.unattended = &old;
    config.playbooks_dir = Some(&rig.cwd);
    config.credential_refresh = Some("eph-gh-token refresh");
    let plan = claude.plan(&config).unwrap();
    let settings = plan.settings().unwrap();
    assert_eq!(
        settings.content()["attribution"],
        json!({"commit":"","pr":"","sessionUrl":false})
    );
    assert_eq!(settings.content()["includeCoAuthoredBy"], false);
    assert!(
        settings.content()["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains(" --dir ")
    );
    assert!(
        settings.content()["permissions"]["allow"]
            .as_array()
            .unwrap()
            .contains(&json!("Bash(cargo check:*)"))
    );
    assert!(
        settings.content()["permissions"]["allow"]
            .as_array()
            .unwrap()
            .contains(&json!("Bash($EPH_GH_TOKEN)"))
    );
    settings.install().unwrap();
    write_atomic(
        &settings.path(),
        br#"{"hooks":{"foreign":[]},"permissions":{"allow":["Bash(old:*)"]}}"#,
    )
    .unwrap();
    let clean_plan = claude.plan(&rig.config(Autonomy::Manual)).unwrap();
    let clean = clean_plan.settings().unwrap();
    clean.install().unwrap();
    let disk: Value = serde_json::from_slice(&fs::read(clean.path()).unwrap()).unwrap();
    assert_eq!(&disk, clean.content());
    assert!(disk["hooks"].get("foreign").is_none());
    assert_eq!(disk["permissions"]["allow"].as_array().unwrap().len(), 2);
    assert_eq!(
        disk["permissions"]["additionalDirectories"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    write_atomic(&settings.path(), b"null").unwrap();
    assert!(clean.install().is_err());
    assert_eq!(fs::read(clean.path()).unwrap(), b"null");
    for (command, prefix) in [
        ("", false),
        ("cargo", true),
        (" cargo test", false),
        ("one; two", false),
        ("run ../path", false),
    ] {
        assert!(UnattendedGrant::new(command, prefix).is_err());
    }
}

#[test]
fn claude_accepts_twelve_unattended_grants_and_refuses_thirteen() {
    let rig = Rig::new(EngineId::Claude);
    let claude = rig.claude();
    let grants: [UnattendedGrant; 13] = std::array::from_fn(|i| {
        UnattendedGrant::new(&format!("cargo run tool{i}"), false).unwrap()
    });
    let mut config = rig.config(Autonomy::Autonomous);
    config.unattended = &grants[..12];
    let plan = claude.plan(&config).unwrap();
    let allow = plan.settings().unwrap().content()["permissions"]["allow"]
        .as_array()
        .unwrap();
    assert!(allow.contains(&json!("Bash(cargo run tool11)")));
    config.unattended = &grants;
    assert!(matches!(
        claude.plan(&config),
        Err(eph_engines::AdapterError::Unattended)
    ));
}

#[test]
fn generating_private_settings_preserves_the_other_name_of_a_hard_link() {
    let rig = Rig::new(EngineId::Claude);
    let plan = rig.claude().plan(&rig.config(Autonomy::Manual)).unwrap();
    let settings = plan.settings().unwrap();
    settings.install().unwrap();
    let previous = br#"{"permissions":{"allow":["Bash(old:*)"]}}"#;
    let peer = rig.root.path().join("previous-settings.json");
    write_atomic(&settings.path(), previous).unwrap();
    fs::hard_link(settings.path(), &peer).unwrap();

    settings.install().unwrap();

    let installed: Value = serde_json::from_slice(&fs::read(settings.path()).unwrap()).unwrap();
    assert_eq!(&installed, settings.content());
    assert_eq!(fs::read(peer).unwrap(), previous);
}

#[test]
fn tool_grants_are_canonical_and_missing_is_reported() {
    let rig = Rig::new(EngineId::Claude);
    let plugin = rig.home.join("plugin");
    fs::create_dir(&plugin).unwrap();
    let roots = ToolRoots {
        home: &rig.home,
        target: &rig.cwd,
    };
    let tools = resolve_tool_grants(
        &[
            ToolGrant {
                root: ToolRoot::Home,
                path: Path::new("plugin"),
            },
            ToolGrant {
                root: ToolRoot::Home,
                path: Path::new("./plugin"),
            },
            ToolGrant {
                root: ToolRoot::Home,
                path: Path::new("within/../plugin"),
            },
            ToolGrant {
                root: ToolRoot::Target,
                path: Path::new("missing"),
            },
        ],
        roots,
    )
    .unwrap();
    assert_eq!(tools.directories(), [fs::canonicalize(&plugin).unwrap()]);
    assert!(tools.missing()[0].ends_with("missing"));
    let mut config = rig.config(Autonomy::Autonomous);
    config.tools = &tools;
    let plan = rig.claude().plan(&config).unwrap();
    assert_eq!(plan.args()[5], "--plugin-dir");
    assert_eq!(plan.args()[6], tools.directories()[0]);
    assert!(
        resolve_tool_grants(
            &[ToolGrant {
                root: ToolRoot::Home,
                path: Path::new("../repo")
            }],
            roots
        )
        .is_err()
    );
}

#[test]
fn home_prompt_override_and_unreadable_override_follow_the_same_entry_point() {
    let rig = Rig::new(EngineId::Claude);
    let claude = rig.claude();
    let dir = rig.prompts.join("engines");
    fs::create_dir(&dir).unwrap();
    let template = dir.join("identity-appendix.md");
    write_atomic(&template, b"{{agentId}} {{identity}} {{memory}}\n\n\n").unwrap();
    let plan = claude.plan(&rig.config(Autonomy::Manual)).unwrap();
    assert_eq!(
        plan.args().last().unwrap(),
        "agent.contract # The hire Remember {{identity}} as data.\n"
    );
    assert!(plan.prompt_condition().is_none());
    write_atomic(&template, b"{{unbound}}").unwrap();
    assert!(claude.plan(&rig.config(Autonomy::Manual)).is_err());
    fs::remove_file(&template).unwrap();
    fs::create_dir(&template).unwrap();
    let plan = claude.plan(&rig.config(Autonomy::Manual)).unwrap();
    assert!(plan.prompt_condition().is_some());
    assert!(
        plan.args()
            .last()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("# The protocol")
    );
}

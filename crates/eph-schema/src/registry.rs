//! `registry.json`: who is hired (SDD-RUST §4.5; FR-4.2).
//!
//! Contract: the registry holds facts about a hire, never a running process's
//! state (SDD-RUST §4.0 rule 7). An agent's status, its engine's hook fidelity
//! and when it was last seen are derived at runtime and shown, not stored.
//! `retiredAt` is the one lifecycle fact routing needs, since mail to a retired
//! agent bounces.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::names::{AgentId, EnvName, Slug, check_len};
use crate::time::Timestamp;

/// An engine a hire runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineId {
    /// Claude Code, the reference engine (ADR-0024).
    Claude,
    /// Codex CLI (a conformance subject).
    Codex,
    /// Gemini CLI (a conformance subject).
    Gemini,
    /// Grok.
    Grok,
    /// OpenCode.
    Opencode,
    /// A custom command.
    Custom,
}

/// A hire's daily budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Budget {
    /// Tokens per day.
    pub daily_tokens: u64,
}

/// The hire template an agent was hired from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HireRef {
    /// The template's name.
    pub template: Slug,
    /// Its version, at least 1.
    pub version: u32,
}

/// One hired agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Agent {
    /// Its display name, 1 to 64 characters.
    pub name: String,
    /// Its role, 1 to 64 characters.
    pub role: String,
    /// Its engine.
    pub engine: EngineId,
    /// Its capabilities, at most 32, each 1 to 64 characters.
    pub capabilities: Vec<String>,
    /// Its seat on the floor, 1 to 32 characters.
    pub seat: String,
    /// The environment variables it is granted, by name (invariant 6).
    pub env_grants: Vec<EnvName>,
    /// The profile instance that hired it, if any.
    pub profile: Option<String>,
    /// The directory its process runs in.
    pub cwd: String,
    /// The target it works on (`repo:x`), if any.
    pub target: Option<String>,
    /// Its budget; absent means unbudgeted (ADR-0029).
    pub budget: Option<Budget>,
    /// The hire template it was hired from. Every hire has one: SDD-RUST §4.5
    /// writes `hire` where today's build never did (inventory P7).
    pub hire: HireRef,
    /// When it was hired.
    pub spawned_at: Timestamp,
    /// When it was retired; absent while it is on the roster.
    pub retired_at: Option<Timestamp>,
}

/// `registry.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Registry {
    /// [`Registry::VERSION`].
    pub schema_version: u32,
    /// The orchestrator, Artemis, when she is hired.
    pub orchestrator_id: Option<AgentId>,
    /// Every agent ever hired into this company, by id.
    pub agents: BTreeMap<AgentId, Agent>,
}

impl Registry {
    /// An empty roster.
    pub fn empty() -> Registry {
        Registry {
            schema_version: Self::VERSION,
            orchestrator_id: None,
            agents: BTreeMap::new(),
        }
    }

    /// The agents on the roster now: hired and not retired.
    pub fn active(&self) -> impl Iterator<Item = (&AgentId, &Agent)> {
        self.agents
            .iter()
            .filter(|(_, agent)| agent.retired_at.is_none())
    }
}

impl Versioned for Registry {
    const FILE: &'static str = "registry.json";
    const VERSION: u32 = 2;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if let Some(orchestrator) = &self.orchestrator_id
            && !self.agents.contains_key(orchestrator)
        {
            return Err(SchemaError::invalid(
                Self::FILE,
                "orchestratorId",
                format!("names `{orchestrator}`, who is not in `agents`"),
            ));
        }
        for (id, agent) in &self.agents {
            let field = |name: &str| format!("agents.{id}.{name}");
            if id.is_reserved() {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("agents.{id}"),
                    "is an id the harness writes mail under; no hire may take it",
                ));
            }
            check_len(Self::FILE, &field("name"), &agent.name, 1, 64)?;
            check_len(Self::FILE, &field("role"), &agent.role, 1, 64)?;
            check_len(Self::FILE, &field("seat"), &agent.seat, 1, 32)?;
            check_len(Self::FILE, &field("cwd"), &agent.cwd, 1, 4096)?;
            if agent.capabilities.len() > 32 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("capabilities"),
                    "at most 32 capabilities",
                ));
            }
            for capability in &agent.capabilities {
                check_len(Self::FILE, &field("capabilities"), capability, 1, 64)?;
            }
            if agent.env_grants.len() > 32 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("envGrants"),
                    "at most 32 grants",
                ));
            }
            if let Some(profile) = &agent.profile {
                check_len(Self::FILE, &field("profile"), profile, 1, 64)?;
            }
            if let Some(target) = &agent.target {
                check_len(Self::FILE, &field("target"), target, 1, 256)?;
            }
            if agent.hire.version == 0 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("hire.version"),
                    "must be at least 1",
                ));
            }
            if agent
                .retired_at
                .is_some_and(|retired| retired < agent.spawned_at)
            {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("retiredAt"),
                    "must not be before spawnedAt",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{parse, render};

    fn agent() -> Agent {
        Agent {
            name: "Mason".into(),
            role: "builder".into(),
            engine: EngineId::Claude,
            capabilities: vec!["rust".into()],
            seat: "desk-1".into(),
            env_grants: vec!["GH_TOKEN".parse().unwrap()],
            profile: Some("crew@repo-x".into()),
            cwd: "C:/work/x".repeat(100),
            target: Some("repo:x".into()),
            budget: None,
            hire: HireRef {
                template: "builder".parse().unwrap(),
                version: 1,
            },
            spawned_at: Timestamp::from_millis(1_000).unwrap(),
            retired_at: None,
        }
    }

    fn registry(agents: Vec<(&str, Agent)>) -> Registry {
        Registry {
            schema_version: 2,
            orchestrator_id: None,
            agents: agents
                .into_iter()
                .map(|(id, a)| (id.parse().unwrap(), a))
                .collect(),
        }
    }

    #[test]
    fn a_roster_round_trips_and_a_long_cwd_fits_now() {
        // Today's `target` held a cwd of up to 4096 characters against a schema
        // of 256 (inventory P3): `cwd` and `target` are separate fields now.
        let roster = registry(vec![("agent.mason", agent())]);
        let text = render(&roster).unwrap();
        assert_eq!(parse::<Registry>(&text).unwrap(), roster);
        assert!(text.contains("\"spawnedAt\": \"1970-01-01T00:00:01.000Z\""));
    }

    #[test]
    fn process_state_is_not_a_field() {
        let mut value: serde_json::Value =
            serde_json::from_str(&render(&registry(vec![("agent.mason", agent())])).unwrap())
                .unwrap();
        value["agents"]["agent.mason"]["status"] = "idle".into();
        let error = parse::<Registry>(&value.to_string())
            .unwrap_err()
            .to_string();
        assert!(error.contains("status"), "{error}");
    }

    #[test]
    fn each_rule_refuses_naming_the_agent() {
        let mut orphan = registry(vec![("agent.mason", agent())]);
        orphan.orchestrator_id = Some("agent.artemis".parse().unwrap());
        assert!(
            render(&orphan)
                .unwrap_err()
                .to_string()
                .contains("not in `agents`")
        );

        assert!(
            render(&registry(vec![("agent.ledger", agent())]))
                .unwrap_err()
                .to_string()
                .contains("harness writes mail under")
        );

        let mut retired_early = agent();
        retired_early.retired_at = Some(Timestamp::from_millis(500).unwrap());
        assert!(
            render(&registry(vec![("agent.mason", retired_early)]))
                .unwrap_err()
                .to_string()
                .contains("retiredAt")
        );

        let mut nameless = agent();
        nameless.name = String::new();
        assert!(
            render(&registry(vec![("agent.mason", nameless)]))
                .unwrap_err()
                .to_string()
                .contains("agents.agent.mason.name")
        );

        let mut zero_hire = agent();
        zero_hire.hire.version = 0;
        assert!(
            render(&registry(vec![("agent.mason", zero_hire)]))
                .unwrap_err()
                .to_string()
                .contains("hire.version")
        );
    }

    #[test]
    fn a_hire_without_its_template_is_refused() {
        let mut value: serde_json::Value =
            serde_json::from_str(&render(&registry(vec![("agent.mason", agent())])).unwrap())
                .unwrap();
        value["agents"]["agent.mason"]
            .as_object_mut()
            .unwrap()
            .remove("hire");
        let error = parse::<Registry>(&value.to_string())
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing field `hire`"), "{error}");
    }

    #[test]
    fn active_skips_the_retired() {
        let mut retired = agent();
        retired.retired_at = Some(Timestamp::from_millis(2_000).unwrap());
        let roster = registry(vec![("agent.a", agent()), ("agent.b", retired)]);
        let active: Vec<&str> = roster.active().map(|(id, _)| id.as_str()).collect();
        assert_eq!(active, ["agent.a"]);
        assert!(Registry::empty().agents.is_empty());
    }

    #[test]
    fn capabilities_and_grants_are_capped_at_32() {
        let mut many = agent();
        many.capabilities = (0..33).map(|i| format!("c{i}")).collect();
        let error = render(&registry(vec![("agent.mason", many)]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("agents.agent.mason.capabilities: at most 32 capabilities"),
            "{error}"
        );
        let mut at_cap = agent();
        at_cap.capabilities = (0..32).map(|i| format!("c{i}")).collect();
        at_cap.env_grants = (0..32)
            .map(|i| format!("GRANT_{i}").parse().unwrap())
            .collect();
        assert!(render(&registry(vec![("agent.mason", at_cap.clone())])).is_ok());
        at_cap.env_grants.push("GRANT_32".parse().unwrap());
        let error = render(&registry(vec![("agent.mason", at_cap)]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("agents.agent.mason.envGrants: at most 32 grants"),
            "{error}"
        );
        let mut blank = agent();
        blank.capabilities = vec![String::new()];
        assert!(
            render(&registry(vec![("agent.mason", blank)]))
                .unwrap_err()
                .to_string()
                .contains("capabilities")
        );
        for (field, edit) in [
            (
                "profile",
                Box::new(|a: &mut Agent| a.profile = Some(String::new()))
                    as Box<dyn Fn(&mut Agent)>,
            ),
            (
                "target",
                Box::new(|a: &mut Agent| a.target = Some("x".repeat(257))),
            ),
            ("role", Box::new(|a: &mut Agent| a.role = String::new())),
            ("seat", Box::new(|a: &mut Agent| a.seat = "s".repeat(33))),
            ("cwd", Box::new(|a: &mut Agent| a.cwd = String::new())),
        ] {
            let mut bad = agent();
            edit(&mut bad);
            let error = render(&registry(vec![("agent.mason", bad)]))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("agents.agent.mason.{field}")),
                "{field}: {error}"
            );
        }
    }

    /// Round 1 (2026-10-20): retiring an agent the moment it was hired is allowed.
    #[test]
    fn an_agent_may_retire_the_instant_it_was_hired() {
        let mut instant = agent();
        instant.retired_at = Some(instant.spawned_at);
        assert!(render(&registry(vec![("agent.mason", instant)])).is_ok());
    }
}

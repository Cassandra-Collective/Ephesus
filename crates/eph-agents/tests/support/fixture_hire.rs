use std::path::Path;

use eph_schema::{
    gates::Autonomy,
    names::AgentId,
    registry::{Agent, EngineId, HireRef},
    time::Timestamp,
};

use crate::{ExitPolicy, Hire};

pub(crate) fn hire(id: &AgentId, target: &Path) -> Hire {
    Hire {
        id: id.clone(),
        roster: Agent {
            name: "Fixture hire".into(),
            role: "Test role".into(),
            engine: EngineId::Claude,
            capabilities: Vec::new(),
            seat: "fixture".into(),
            env_grants: vec!["FIXTURE_GRANT".parse().unwrap()],
            profile: None,
            cwd: target.display().to_string(),
            target: None,
            budget: None,
            hire: HireRef {
                template: "fixture".parse().unwrap(),
                version: 1,
            },
            spawned_at: Timestamp::now(),
            retired_at: None,
        },
        target: target.to_path_buf(),
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

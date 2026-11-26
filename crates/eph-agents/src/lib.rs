//! The agent lifecycle and command queue (SDD-RUST §3.4, ADR-0045).

mod commands;
mod journal;
mod launch;
mod manager;
mod respawn;

#[cfg(test)]
mod fixture_hire {
    include!("../tests/support/fixture_hire.rs");
}
#[cfg(test)]
mod scratch {
    include!("../tests/support/scratch.rs");
}

pub use commands::{CommandPhase, HeldCommand};
pub use launch::DeclaredTool;
pub use manager::{Manager, TerminalSize};
pub use respawn::ExitPolicy;

use std::{fmt, io, path::PathBuf};

use eph_engines::{CommandSpec, UnattendedGrant};
use eph_schema::{
    gates::Autonomy,
    lifecycle::TerminalKind,
    names::{AgentId, EnvName},
    registry::Agent,
};

/// The company identity supplied by the launcher (ADR-0022).
pub struct CompanyIdentity {
    /// Git author and committer name.
    pub name: String,
    /// Git author and committer email.
    pub email: String,
}

/// One declared hire and the activation input for its engine launches.
pub struct Hire {
    /// The roster key and the owner of the corresponding resources.
    pub id: AgentId,
    /// The durable hire facts, without live status.
    pub roster: Agent,
    /// The workspace approved by the caller before optional isolation.
    pub target: PathBuf,
    /// Whether the hire requires its own worktree.
    pub isolated: bool,
    /// The composed autonomy requested for the engine.
    pub autonomy: Autonomy,
    /// The identity material already composed by the caller.
    pub identity: String,
    /// The hire's declared continuation policy.
    pub exit_policy: ExitPolicy,
    /// Explicit tool directories for the engine.
    pub tools: Vec<DeclaredTool>,
    /// The optional read-only directory of runbooks.
    pub playbooks: Option<PathBuf>,
    /// The declared unattended command grants.
    pub unattended: Vec<UnattendedGrant>,
    /// The optional company commit identity.
    pub commit_identity: Option<CompanyIdentity>,
    /// The optional recall command supplied by its owner.
    pub recall_command: Option<String>,
    /// The optional credential refresh command supplied by its owner.
    pub credential_refresh: Option<String>,
}

/// Identifies one current launch in this manager's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generation(u64);

/// The authorization created by the caller's hook registration boundary.
pub struct HookRegistration {
    /// The current launch's credential.
    pub token: String,
    /// The endpoint that owns the registration.
    pub endpoint: String,
}

/// Collaborators owned by later phases. Inputs are for the exact hire and launch.
pub trait Services {
    /// Resolve one declared secret. An absent value is reported by name.
    fn grant(&mut self, agent: &AgentId, name: &EnvName) -> Result<Option<String>, Error>;

    /// Read the caller's current memory text for this hire.
    fn memory(&mut self, agent: &AgentId) -> Result<String, Error>;

    /// Register a fresh launch token before returning. An error leaves no registration.
    fn register(
        &mut self,
        agent: &AgentId,
        generation: Generation,
    ) -> Result<HookRegistration, Error>;

    /// Revoke the exact registration. Retrying revocation is safe.
    fn revoke(&mut self, agent: &AgentId, generation: Generation) -> Result<(), Error>;

    /// Notify the caller that an engine session has ended and its deliveries can return.
    fn ended(&mut self, _agent: &AgentId, _generation: Generation) -> Result<(), Error> {
        Ok(())
    }

    /// Confirm whether the observed engine session still has a transcript.
    /// None means the caller cannot establish presence.
    fn transcript_present(&mut self, _agent: &AgentId, _session: &str) -> Option<bool> {
        None
    }
}

/// Current public lifecycle of a hire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifecycle {
    /// The manager is preparing the launch.
    Checking,
    /// The exact install offered for this launch, awaiting consent.
    InstallOffered(CommandSpec),
    /// The installer owns the visible terminal.
    Installing,
    /// The engine's probe established logout and supplied this remedy.
    NeedsLogin(CommandSpec),
    /// The engine and its successful launch record are live.
    Running,
    /// An earlier attempt still owns resources.
    Cleaning,
    /// A live session ended and can be considered for respawn.
    Exited,
    /// A launch could not proceed.
    Refused(String),
    /// The operator or harness cancelled an attempt.
    Stopped,
}

/// An observation for the current manager caller.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// Bytes from one redacted terminal stream.
    Output {
        /// The hire that owns the stream.
        agent: AgentId,
        /// The launch that owns the stream.
        generation: Generation,
        /// The terminal within that launch.
        terminal: TerminalKind,
        /// The redacted bytes, with terminal controls intact.
        bytes: Vec<u8>,
    },
    /// A visible condition associated with a hire.
    Problem {
        /// The affected hire.
        agent: AgentId,
        /// A safe description of what needs attention.
        detail: String,
    },
}

/// A refused manager request or a typed boundary failure.
#[derive(Debug)]
pub enum Error {
    /// No manager entry exists for the hire.
    Unknown(AgentId),
    /// An attempt still owns resources or the request is from an older launch.
    Busy(AgentId),
    /// Authority or lifecycle state refused the request.
    Refused(String),
    /// A filesystem, worker, or terminal operation failed.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(agent) => write!(f, "unknown hire {agent}"),
            Self::Busy(agent) => {
                write!(f, "{agent} already has an attempt or the request is stale")
            }
            Self::Refused(reason) => f.write_str(reason),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

//! The R2.3 lifecycle vocabulary of the Agora's book (SDD-RUST §3.4).

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    SchemaError,
    document::Versioned,
    id::TaskId,
    log::{LogKind, LogLine},
    names::{AgentId, EnvName},
    registry::EngineId,
};

pub(crate) const SPAWN: &[&str] = &[
    "requested",
    "install-offered",
    "install-accepted",
    "install-exited",
    "started",
    "refused",
];
pub(crate) const EXIT: &[&str] = &["observed", "unwound"];
pub(crate) const GHOST: &[&str] = &["offered"];
pub(crate) const RESPAWN: &[&str] = &[
    "scheduled",
    "held",
    "deferred",
    "released",
    "blocked",
    "respawned",
    "failed",
    "exhausted",
];
pub(crate) const TASK: &[&str] = &["returned"];

/// The terminal that produced an exit report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalKind {
    /// The explicitly accepted installation command.
    Installer,
    /// The agent engine.
    Engine,
}

/// An observed process result, or the reason observation failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ProcessResult {
    /// An ordinary program exit.
    Code {
        /// The reported code.
        code: u32,
    },
    /// An OS signal ended the program.
    Signal {
        /// Reported signal name or number.
        signal: String,
    },
    /// The status could not be confirmed.
    Unconfirmed {
        /// Why a result was unavailable.
        reason: String,
    },
}

/// What became of a checkout leased to the attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CheckoutResult {
    /// No checkout was acquired.
    None,
    /// Git unregistered the checkout.
    Removed {
        /// Anything left behind after unregistering.
        residue: Option<String>,
    },
    /// The checkout was preserved.
    Retained {
        /// Why cleanup retained the checkout.
        reason: String,
        /// Worktree evidence from Git, if available.
        changes: Vec<String>,
    },
    /// Terminal ownership or checkout cleanup remains outstanding.
    Unconfirmed {
        /// What still holds the checkout.
        reason: String,
    },
}

/// A literal install offer, including the approved installer directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallCommand {
    /// Program name from the adapter.
    pub program: String,
    /// Literal arguments following the program name.
    pub args: Vec<String>,
    /// Harness-owned working directory.
    pub cwd: String,
}

/// The result of verifying an installer outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InstallVerification {
    /// The installer did not complete successfully.
    Skipped,
    /// The installed executable reported its version.
    Verified {
        /// The version it reported.
        version: String,
    },
    /// The executable still could not run with a recognized version.
    Refused {
        /// The reason it could not be verified.
        reason: String,
    },
}

/// Typed payloads for the manager's events, before stamping the book envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum LifecycleEvent {
    /// A hire's new launch request.
    Requested {
        /// The requested engine.
        engine: EngineId,
        /// The approved target before optional isolation.
        workspace: String,
        /// Declared environment grant names.
        grants: Vec<EnvName>,
    },
    /// Confirmed absence yielded an offer.
    InstallOffered(InstallCommand),
    /// The Architect accepted this offer.
    InstallAccepted(InstallCommand),
    /// The installer exited and, if successful, was verified.
    InstallExited {
        /// The installer's process result.
        status: ProcessResult,
        /// The verification after exit.
        verification: InstallVerification,
    },
    /// The engine PTY was acquired.
    Started {
        /// The recognized engine version.
        version: String,
        /// The actual engine working directory.
        workspace: String,
    },
    /// Launch could not proceed.
    Refused {
        /// A reason safe for the book.
        reason: String,
    },
    /// A terminal ended or its shutdown could not be confirmed.
    Observed {
        /// Which terminal produced this result.
        terminal: TerminalKind,
        /// The terminal's process result.
        status: ProcessResult,
    },
    /// Per-hire teardown outcomes.
    Unwound {
        /// What became of the checkout.
        checkout: CheckoutResult,
        /// Teardown failures, if any.
        failures: Vec<String>,
    },
    /// The operator can bring a terminated agent back.
    Offered {
        /// The observed engine result.
        status: ProcessResult,
        /// Whether the session can be resumed.
        resumable: bool,
        /// Whether the memory caller has continuity to offer.
        memory: bool,
        /// In-flight tasks returned to the ledger.
        tasks: Vec<TaskId>,
        /// Current standing stop, if any.
        stop: Option<String>,
        /// Current capacity disposition, if held.
        capacity: Option<String>,
    },
    /// A respawn rung acquired a deadline.
    Scheduled {
        /// One-based rung number.
        rung: usize,
        /// Wait in milliseconds.
        wait_ms: u64,
    },
    /// Capacity placed a hold on the ladder.
    Held {
        /// Rungs already spent.
        rung: usize,
        /// The current hold reason.
        reason: String,
    },
    /// A due respawn was deferred without spending a rung.
    Deferred {
        /// Rungs already spent.
        rung: usize,
        /// The reason for the deferral.
        reason: String,
    },
    /// Capacity released a hold.
    Released {
        /// Rungs already spent.
        rung: usize,
    },
    /// Durable authority refused a respawn.
    Blocked {
        /// Why a new attempt cannot proceed.
        reason: String,
    },
    /// A respawn reached a live engine terminal.
    Respawned {
        /// The rung that started the engine.
        rung: usize,
    },
    /// The rung tried and did not restore a live engine.
    Failed {
        /// The rung that was spent.
        rung: usize,
        /// What failed.
        reason: String,
    },
    /// The bounded ladder is spent.
    Exhausted {
        /// How many rungs it spent.
        rung: usize,
        /// What remains for the operator to decide.
        reason: String,
    },
    /// Unfinished tasks went back to `todo`.
    Returned {
        /// The tasks that changed.
        tasks: Vec<TaskId>,
        /// Why these tasks were returned.
        reason: ReturnReason,
    },
}

/// The reason the manager returns an exited hire's unfinished work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReturnReason {
    /// The agent process ended.
    AgentExit,
}

impl LifecycleEvent {
    fn kind(&self) -> LogKind {
        match self {
            Self::Requested { .. }
            | Self::InstallOffered(_)
            | Self::InstallAccepted(_)
            | Self::InstallExited { .. }
            | Self::Started { .. }
            | Self::Refused { .. } => LogKind::Spawn,
            Self::Observed { .. } | Self::Unwound { .. } => LogKind::Exit,
            Self::Offered { .. } => LogKind::Ghost,
            Self::Returned { .. } => LogKind::Task,
            _ => LogKind::Respawn,
        }
    }
}

/// A typed lifecycle event associated with a hire and its acknowledged request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleRecord {
    /// The hire the event concerns.
    pub agent: AgentId,
    /// The book sequence of `spawn/requested`, when it exists.
    pub attempt: Option<NonZeroU64>,
    /// The typed lifecycle event.
    pub event: LifecycleEvent,
}

/// Encoded lifecycle fields for the book to stamp and append.
pub struct LifecycleFields {
    /// The envelope kind.
    pub kind: LogKind,
    /// The event name within that kind.
    pub event: String,
    /// The hire's validated identifier.
    pub source: String,
    /// The event-specific payload and its correlation.
    pub data: Map<String, Value>,
}

impl LifecycleRecord {
    /// Encode a typed event for the existing book envelope.
    pub fn fields(&self) -> Result<LifecycleFields, SchemaError> {
        #[derive(Deserialize)]
        struct Body {
            event: String,
            data: Map<String, Value>,
        }
        let encoded = serde_json::to_value(&self.event).map_err(invalid_event)?;
        let Body { event, mut data } = serde_json::from_value(encoded).map_err(invalid_event)?;
        if let Some(attempt) = self.attempt {
            data.insert("attempt".into(), Value::from(attempt.get()));
        }
        Ok(LifecycleFields {
            kind: self.event.kind(),
            event,
            source: self.agent.to_string(),
            data,
        })
    }
}

pub(crate) fn validate(line: &LogLine) -> Result<(), SchemaError> {
    let source = line
        .source
        .as_deref()
        .and_then(|source| source.parse::<AgentId>().ok());
    if source.is_none() {
        return Err(SchemaError::invalid(
            LogLine::FILE,
            "source",
            "a lifecycle event requires a hire id",
        ));
    }
    let mut data = line.data.clone();
    let attempt = data
        .remove("attempt")
        .map(serde_json::from_value::<NonZeroU64>)
        .transpose()
        .map_err(invalid_event)?;
    let event: LifecycleEvent = serde_json::from_value(serde_json::json!({
        "event": line.event,
        "data": data,
    }))
    .map_err(invalid_event)?;
    // LogLine validates the kind/event pair against its closed list first.
    let correlated = if matches!(event, LifecycleEvent::Requested { .. }) {
        attempt.is_none()
    } else {
        attempt.is_some_and(|attempt| attempt.get() < line.seq)
    };
    if !correlated {
        return Err(SchemaError::invalid(
            LogLine::FILE,
            "data.attempt",
            "a request starts its attempt; every follow-up requires an earlier request sequence",
        ));
    }
    Ok(())
}

fn invalid_event(error: serde_json::Error) -> SchemaError {
    SchemaError::invalid(
        LogLine::FILE,
        "data",
        format!("invalid lifecycle data: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LineRead, read_line, render_line};

    fn line(event: LifecycleEvent, attempt: Option<u64>) -> LogLine {
        let fields = LifecycleRecord {
            agent: "agent.fixture".parse().unwrap(),
            attempt: attempt.and_then(NonZeroU64::new),
            event,
        }
        .fields()
        .unwrap();
        LogLine {
            schema_version: LogLine::VERSION,
            seq: 4,
            ts: crate::time::Timestamp::from_millis(1_000).unwrap(),
            kind: fields.kind,
            event: fields.event,
            source: Some(fields.source),
            refs: Vec::new(),
            data: fields.data,
        }
    }

    #[test]
    fn follow_ups_round_trip_and_require_a_prior_attempt() {
        let event = LifecycleEvent::Started {
            version: "1.2.3".into(),
            workspace: "/fixture".into(),
        };
        let first = line(event.clone(), Some(3));
        assert_eq!(
            read_line(&render_line(&first).unwrap()),
            LineRead::Entry(first)
        );
        for attempt in [None, Some(4), Some(5)] {
            let rejected = render_line(&line(event.clone(), attempt))
                .unwrap_err()
                .to_string();
            assert!(rejected.contains("data.attempt"), "{rejected}");
        }
        let request = LifecycleEvent::Requested {
            engine: EngineId::Claude,
            workspace: "/fixture".into(),
            grants: Vec::new(),
        };
        assert!(render_line(&line(request.clone(), None)).is_ok());
        assert!(render_line(&line(request, Some(1))).is_err());
    }

    #[test]
    fn known_events_refuse_a_missing_hire_or_incorrect_payload() {
        let mut first = line(
            LifecycleEvent::Observed {
                terminal: TerminalKind::Engine,
                status: ProcessResult::Signal {
                    signal: "test".into(),
                },
            },
            Some(1),
        );
        first.source = None;
        assert!(
            render_line(&first)
                .unwrap_err()
                .to_string()
                .contains("requires a hire id")
        );
        first.source = Some("agent.fixture".into());
        first.data.insert("status".into(), serde_json::json!(17));
        assert!(
            render_line(&first)
                .unwrap_err()
                .to_string()
                .contains("invalid lifecycle data")
        );
    }

    #[test]
    fn each_lifecycle_kind_reads_its_own_typed_event() {
        let cases = [
            (
                LogKind::Ghost,
                LifecycleEvent::Offered {
                    status: ProcessResult::Code { code: 0 },
                    resumable: false,
                    memory: false,
                    tasks: Vec::new(),
                    stop: None,
                    capacity: None,
                },
                "offered",
            ),
            (
                LogKind::Respawn,
                LifecycleEvent::Scheduled {
                    rung: 1,
                    wait_ms: 5_000,
                },
                "scheduled",
            ),
            (
                LogKind::Task,
                LifecycleEvent::Returned {
                    tasks: Vec::new(),
                    reason: ReturnReason::AgentExit,
                },
                "returned",
            ),
        ];
        for (kind, event, name) in cases {
            let row = line(event, Some(3));
            assert_eq!(row.kind, kind);
            assert_eq!(row.event, name);
            assert_eq!(read_line(&render_line(&row).unwrap()), LineRead::Entry(row));
        }
    }
}

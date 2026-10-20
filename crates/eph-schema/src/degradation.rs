//! The degradation channel's row: the payload of a `degradation` log line
//! (SDD-RUST §4.5; invariant 7).
//!
//! Contract: one row per CAUSE, not per occurrence. A cause is
//! `<source>/<slug>[:<subject>]` (`restart/orphan-block:t-…`), and the row's
//! source is its cause's, never a second field that could disagree (SDD-RUST P7;
//! review finding F5). Whether a line raises or clears is likewise not chosen
//! by the writer: a row that carries `clearedAfter` is a clear, one that does
//! not is a raise, and a line whose `event` says otherwise is refused. Rows
//! reach the book of record on a bounded ladder ([`earns_log_line`]): the first
//! occurrence, then every power of ten, then the clear.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::SchemaError;
use crate::log::{LogKind, LogLine};
use crate::names::check_len;
use crate::time::{Duration, Timestamp};

/// The events a degradation line may carry.
pub const EVENTS: &[&str] = &["raised", "cleared"];

/// The subsystem a degradation belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Agent lifecycle.
    Agents,
    /// The Agora.
    Agora,
    /// The orchestrator.
    Artemis,
    /// Autonomy enforcement.
    Autonomy,
    /// The circuit breaker.
    Breaker,
    /// Budgets.
    Budgets,
    /// Engine capacity.
    Capacity,
    /// Text the harness typed that the agent never ran.
    Commands,
    /// The company waits for the Architect's go (ADR-0032).
    Consent,
    /// The control socket.
    Control,
    /// Engine adapters.
    Engines,
    /// Gates.
    Gates,
    /// The Harbor.
    Harbor,
    /// Hermes.
    Hermes,
    /// The hook endpoint.
    Hooks,
    /// The harness home.
    Home,
    /// Incident handling.
    Incident,
    /// The task ledger.
    Ledger,
    /// The Library.
    Library,
    /// The Odeon.
    Odeon,
    /// Mission profiles.
    Profiles,
    /// The respawn ladder.
    Respawn,
    /// The boot replay (ADR-0027).
    Restart,
    /// The scheduler.
    Scheduler,
    /// The secret broker.
    Secrets,
    /// Settings files written into agents' engines.
    Settings,
    /// The quit sequence.
    Shutdown,
    /// Engine usage reports.
    Usage,
    /// The window (the TypeScript build called it the renderer).
    Window,
}

/// What a degradation is: `<source>/<slug>[:<subject>]`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cause {
    /// The subsystem.
    pub source: Source,
    /// What went wrong, as a lowercase name.
    pub slug: String,
    /// What it went wrong for (a file, an agent, a task), if any.
    pub subject: Option<String>,
}

/// A text that is not a cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadCause(pub String);

impl fmt::Display for BadCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a cause: write <source>/<slug>[:<subject>], like home/seeded-config:gate-policy.json",
            self.0
        )
    }
}

impl FromStr for Cause {
    type Err = BadCause;

    fn from_str(text: &str) -> Result<Cause, BadCause> {
        let bad = || BadCause(text.to_string());
        let (source, rest) = text.split_once('/').ok_or_else(bad)?;
        let source: Source = serde_json::from_value(serde_json::Value::String(source.to_string()))
            .map_err(|_| bad())?;
        let (slug, subject) = match rest.split_once(':') {
            Some((slug, subject)) => (slug, Some(subject)),
            None => (rest, None),
        };
        let slug_ok = slug.parse::<crate::names::Slug>().is_ok();
        let subject_ok = subject.is_none_or(|s| {
            !s.is_empty() && s.chars().count() <= 256 && !s.chars().any(char::is_control)
        });
        if !(slug_ok && subject_ok) {
            return Err(bad());
        }
        Ok(Cause {
            source,
            slug: slug.to_string(),
            subject: subject.map(str::to_string),
        })
    }
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = serde_json::to_value(self.source)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        write!(f, "{source}/{}", self.slug)?;
        if let Some(subject) = &self.subject {
            write!(f, ":{subject}")?;
        }
        Ok(())
    }
}

impl Serialize for Cause {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Cause {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Cause, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|error: BadCause| serde::de::Error::custom(error))
    }
}

/// Whether a degradation line is raised or cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradationEvent {
    /// The condition holds (its first occurrence, or a step on the ladder).
    Raised,
    /// It no longer holds.
    Cleared,
}

impl DegradationEvent {
    /// The event's name on a log line.
    pub fn name(self) -> &'static str {
        match self {
            DegradationEvent::Raised => "raised",
            DegradationEvent::Cleared => "cleared",
        }
    }
}

/// One degradation, as the book of record carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Degradation {
    /// What it is, which names the subsystem it belongs to.
    pub cause: Cause,
    /// The line the Architect reads, 1 to 2000 characters.
    pub detail: String,
    /// How many times it has occurred, from 1.
    pub count: u64,
    /// When it first occurred.
    pub since: Timestamp,
    /// On a clear, and only on a clear: how long it held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_after: Option<Duration>,
}

const FILE: &str = "log.jsonl (degradation)";

impl Degradation {
    /// The subsystem it belongs to: its cause's.
    pub fn source(&self) -> Source {
        self.cause.source
    }

    /// Whether its line raises or clears: a clear is the row that says how long
    /// the condition held.
    pub fn event(&self) -> DegradationEvent {
        if self.cleared_after.is_some() {
            DegradationEvent::Cleared
        } else {
            DegradationEvent::Raised
        }
    }

    /// Every rule a degradation row holds.
    pub fn validate(&self) -> Result<(), SchemaError> {
        check_len(FILE, "detail", &self.detail, 1, 2000)?;
        if self.count == 0 {
            return Err(SchemaError::invalid(FILE, "count", "starts at 1"));
        }
        Ok(())
    }

    /// The log line that carries this row, its event being [`Degradation::event`].
    pub fn to_line(&self, seq: u64, ts: Timestamp) -> Result<LogLine, SchemaError> {
        self.validate()?;
        let data = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => {
                return Err(SchemaError::invalid(
                    FILE,
                    "data",
                    "a degradation renders as an object",
                ));
            }
        };
        Ok(LogLine {
            schema_version: <LogLine as crate::document::Versioned>::VERSION,
            seq,
            ts,
            kind: LogKind::Degradation,
            event: self.event().name().to_string(),
            source: None,
            refs: Vec::new(),
            data,
        })
    }

    /// The row a degradation line carries. A line whose `event` is not the
    /// row's own ([`Degradation::event`]) is refused.
    pub fn from_line(line: &LogLine) -> Result<Degradation, SchemaError> {
        if line.kind != LogKind::Degradation {
            return Err(SchemaError::invalid(
                FILE,
                "kind",
                "is not a degradation line",
            ));
        }
        let row: Degradation = serde_json::from_value(serde_json::Value::Object(line.data.clone()))
            .map_err(|error| SchemaError::Shape {
                file: FILE,
                detail: error.to_string(),
            })?;
        row.validate()?;
        if line.event != row.event().name() {
            return Err(SchemaError::invalid(
                FILE,
                "event",
                "a `cleared` line carries `clearedAfter` and a `raised` line does not",
            ));
        }
        Ok(row)
    }
}

/// Whether the `count`th occurrence earns a line in the book of record: the
/// first, then every power of ten. Every occurrence stays countable in the
/// channel; only the ladder's steps are written.
pub fn earns_log_line(count: u64) -> bool {
    let mut step = 1u64;
    loop {
        if count == step {
            return true;
        }
        match step.checked_mul(10) {
            Some(next) if next <= count => step = next,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{LineRead, read_line, render_line};

    fn row() -> Degradation {
        Degradation {
            cause: "restart/orphan-block:t-20251019T123456789Z-1a2b3c4d"
                .parse()
                .unwrap(),
            detail: "task t-… is blocked by a gate that did not come back".into(),
            count: 1,
            since: Timestamp::from_millis(9_000).unwrap(),
            cleared_after: None,
        }
    }

    #[test]
    fn a_cause_is_source_slug_and_an_optional_subject() {
        let cause: Cause = "home/seeded-config:gate-policy.json".parse().unwrap();
        assert_eq!(cause.source, Source::Home);
        assert_eq!(cause.subject.as_deref(), Some("gate-policy.json"));
        assert_eq!(cause.to_string(), "home/seeded-config:gate-policy.json");
        assert_eq!(
            "window/gone".parse::<Cause>().unwrap().to_string(),
            "window/gone"
        );
        for bad in ["home", "nowhere/x", "home/Bad", "home/x:", "/x", "home/"] {
            let error = bad.parse::<Cause>().unwrap_err();
            assert!(error.to_string().contains("<source>/<slug>"), "{bad}");
        }
    }

    #[test]
    fn a_row_rides_a_degradation_line_and_comes_back() {
        let line = row()
            .to_line(3, Timestamp::from_millis(9_000).unwrap())
            .unwrap();
        let text = render_line(&line).unwrap();
        let LineRead::Entry(read) = read_line(&text) else {
            panic!("{text}")
        };
        assert_eq!(Degradation::from_line(&read).unwrap(), row());
        let mut cleared = row();
        cleared.cleared_after = Some("90s".parse().unwrap());
        let line = cleared
            .to_line(4, Timestamp::from_millis(99_000).unwrap())
            .unwrap();
        assert_eq!(line.event, "cleared");
        assert_eq!(
            cleared.source(),
            Source::Restart,
            "the source is the cause's"
        );
        assert_eq!(Degradation::from_line(&line).unwrap(), cleared);
    }

    #[test]
    fn each_row_rule_refuses() {
        let mut raised = row()
            .to_line(1, Timestamp::from_millis(1).unwrap())
            .unwrap();
        assert_eq!(raised.event, "raised");
        raised.event = "cleared".into();
        let error = Degradation::from_line(&raised).unwrap_err().to_string();
        assert!(
            error.contains("a `cleared` line carries `clearedAfter`"),
            "{error}"
        );
        let mut cleared = row();
        cleared.cleared_after = Some("1s".parse().unwrap());
        let mut line = cleared
            .to_line(2, Timestamp::from_millis(2).unwrap())
            .unwrap();
        line.event = "raised".into();
        assert!(
            Degradation::from_line(&line).is_err(),
            "a raise cannot say how long it held"
        );
        let mut zero = row();
        zero.count = 0;
        assert!(
            zero.validate()
                .unwrap_err()
                .to_string()
                .contains("starts at 1")
        );
        let mut silent = row();
        silent.detail = String::new();
        assert!(
            silent
                .validate()
                .unwrap_err()
                .to_string()
                .contains("detail")
        );
        let mut gym_line = row()
            .to_line(1, Timestamp::from_millis(1).unwrap())
            .unwrap();
        gym_line.kind = LogKind::Gym;
        assert!(
            Degradation::from_line(&gym_line)
                .unwrap_err()
                .to_string()
                .contains("not a degradation line")
        );
    }

    #[test]
    fn the_ladder_writes_the_first_occurrence_and_every_power_of_ten() {
        let earned: Vec<u64> = (1..=1001).filter(|&n| earns_log_line(n)).collect();
        assert_eq!(earned, [1, 10, 100, 1000]);
        assert!(!earns_log_line(0));
        assert!(earns_log_line(10_000_000_000_000_000_000));
        assert!(!earns_log_line(u64::MAX));
    }

    #[test]
    fn a_degradation_line_whose_data_is_not_a_row_is_refused() {
        let mut line = row()
            .to_line(1, Timestamp::from_millis(1).unwrap())
            .unwrap();
        line.data
            .insert("surprise".into(), serde_json::Value::Bool(true));
        let error = Degradation::from_line(&line).unwrap_err();
        assert!(matches!(error, SchemaError::Shape { .. }), "{error:?}");
        assert!(error.to_string().contains("surprise"), "{error}");
        line.data.remove("surprise");
        line.data.insert("count".into(), serde_json::Value::from(0));
        assert!(
            Degradation::from_line(&line)
                .unwrap_err()
                .to_string()
                .contains("starts at 1")
        );
    }
}

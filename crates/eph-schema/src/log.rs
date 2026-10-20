//! A line of the book of record, `log.jsonl` (SDD-RUST §4.5; FR-4.4, NFR-13).
//!
//! Contract:
//! - **The writer is typed.** A line's `kind` comes from [`LogKind`] and its
//!   `event` from that kind's closed list ([`LogKind::events`]), so the writer
//!   cannot emit a pair no reader knows. A kind's events are defined by the
//!   phase that builds its subsystem; until then the kind cannot be written.
//! - **The reader is tolerant.** [`read_line`] keeps a line it does not know (a
//!   newer version, a kind or an event from a newer build) as
//!   [`LineRead::Unknown`] to be counted, never dropped. It reports a line it
//!   cannot read as [`LineRead::Unreadable`]; whether that is a torn last line
//!   or damage in the middle is the log reader's call, by position.
//! - **`seq` identifies a line.** The writer assumes one harness per home
//!   (ADR-0034 is a probe, not a lock), so readers do not trust `seq` to be
//!   unique and keep a position-based cursor.

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::refs::Ref;
use crate::time::Timestamp;

/// What a line is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogKind {
    /// Mail moved between states.
    Delivery,
    /// Mail refused delivery.
    Bounce,
    /// An agent's process started.
    Spawn,
    /// An agent's process ended.
    Exit,
    /// An agent went quiet past its grace.
    Ghost,
    /// A hook event.
    Hook,
    /// The task ledger changed.
    Task,
    /// A gate opened or settled.
    Gate,
    /// A decision memo.
    Memo,
    /// A standup brief.
    Brief,
    /// A slide deck.
    Deck,
    /// A live meeting.
    Meeting,
    /// The circuit breaker.
    Breaker,
    /// Budgets and pacing.
    Budget,
    /// The orchestrator's lifecycle.
    Orchestrator,
    /// An act from outside the window (FR-10.3).
    Remote,
    /// A secret was set or rotated (its name only, never its value).
    SecretRotated,
    /// Mission profiles.
    Profile,
    /// The Gymnasium.
    Gym,
    /// The Stoa.
    Stoa,
    /// The quit sequence.
    Shutdown,
    /// Engine capacity.
    Capacity,
    /// The respawn ladder.
    Respawn,
    /// An error a subsystem could not handle.
    Error,
    /// The degradation channel (SDD-RUST §4.5).
    Degradation,
}

impl LogKind {
    /// Every kind.
    pub const ALL: [LogKind; 25] = [
        LogKind::Delivery,
        LogKind::Bounce,
        LogKind::Spawn,
        LogKind::Exit,
        LogKind::Ghost,
        LogKind::Hook,
        LogKind::Task,
        LogKind::Gate,
        LogKind::Memo,
        LogKind::Brief,
        LogKind::Deck,
        LogKind::Meeting,
        LogKind::Breaker,
        LogKind::Budget,
        LogKind::Orchestrator,
        LogKind::Remote,
        LogKind::SecretRotated,
        LogKind::Profile,
        LogKind::Gym,
        LogKind::Stoa,
        LogKind::Shutdown,
        LogKind::Capacity,
        LogKind::Respawn,
        LogKind::Error,
        LogKind::Degradation,
    ];

    /// The events a line of this kind may carry. Empty means the kind's
    /// vocabulary has not been defined yet: its phase defines it, and until then
    /// no line of it can be written.
    pub fn events(self) -> &'static [&'static str] {
        match self {
            LogKind::Degradation => crate::degradation::EVENTS,
            _ => &[],
        }
    }
}

/// One line of the book of record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LogLine {
    /// [`LogLine::VERSION`].
    pub schema_version: u32,
    /// Its sequence number, from 1.
    pub seq: u64,
    /// When it was written.
    pub ts: Timestamp,
    /// What it is about.
    pub kind: LogKind,
    /// What happened, from the kind's closed list.
    pub event: String,
    /// Who or what it concerns (an agent id, a subsystem), if not the harness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// What it points at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<Ref>,
    /// The event's payload, typed by its kind's module. Always present, as
    /// SDD-RUST §4.5 writes it, even when it is empty.
    pub data: serde_json::Map<String, serde_json::Value>,
}

impl Versioned for LogLine {
    const FILE: &'static str = "log.jsonl";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if self.seq == 0 {
            return Err(SchemaError::invalid(Self::FILE, "seq", "starts at 1"));
        }
        if !self.kind.events().contains(&self.event.as_str()) {
            let allowed = self.kind.events();
            let rule = if allowed.is_empty() {
                "this kind's events are not defined yet; the phase that builds its subsystem defines them".to_string()
            } else {
                format!("must be one of: {}", allowed.join(", "))
            };
            return Err(SchemaError::invalid(Self::FILE, "event", rule));
        }
        if let Some(source) = &self.source {
            let ok = !source.is_empty()
                && source.chars().count() <= 64
                && !source.chars().any(|c| c.is_control() || c.is_whitespace());
            if !ok {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    "source",
                    "1 to 64 characters with no spaces or control characters",
                ));
            }
        }
        Ok(())
    }
}

/// Validates a line and renders it as one line of compact JSON, without the
/// newline the appender adds.
pub fn render_line(line: &LogLine) -> Result<String, SchemaError> {
    line.validate()?;
    serde_json::to_string(line).map_err(|error| SchemaError::Shape {
        file: LogLine::FILE,
        detail: error.to_string(),
    })
}

/// What a reader made of one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineRead {
    /// A line this build knows.
    Entry(LogLine),
    /// A line a newer build wrote, or of a kind or event this build does not
    /// know: kept, counted, never dropped.
    Unknown {
        /// Its sequence number, when it has one.
        seq: Option<u64>,
        /// Why it is not known.
        reason: String,
    },
    /// A line that cannot be read: torn, or damaged.
    Unreadable {
        /// Why.
        reason: String,
    },
}

/// Reads one line of `log.jsonl`. See the module contract.
pub fn read_line(text: &str) -> LineRead {
    let value = match crate::json::strict_value(text) {
        Ok(value) => value,
        Err(error) => {
            return LineRead::Unreadable {
                reason: format!("not JSON, or a key written twice ({error})"),
            };
        }
    };
    let seq = value.get("seq").and_then(serde_json::Value::as_u64);
    match value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        None => {
            return LineRead::Unreadable {
                reason: "no schemaVersion".to_string(),
            };
        }
        Some(v) if v > u64::from(LogLine::VERSION) => {
            return LineRead::Unknown {
                seq,
                reason: format!(
                    "schemaVersion {v}, newer than this build's {}",
                    LogLine::VERSION
                ),
            };
        }
        Some(v) if v < u64::from(LogLine::VERSION) => {
            return LineRead::Unreadable {
                reason: format!("schemaVersion {v}, older than any this build reads"),
            };
        }
        Some(_) => {}
    }
    // A kind or event a newer build added is kept as unknown; one that is
    // missing or not text is damage, never read as something benign (SDD-RUST
    // P5; review finding F6).
    let (Some(kind_text), Some(event)) = (
        value.get("kind").and_then(serde_json::Value::as_str),
        value.get("event").and_then(serde_json::Value::as_str),
    ) else {
        return LineRead::Unreadable {
            reason: "`kind` and `event` must both be present as text".to_string(),
        };
    };
    let kind: Option<LogKind> =
        serde_json::from_value(serde_json::Value::String(kind_text.to_string())).ok();
    let Some(kind) = kind else {
        return LineRead::Unknown {
            seq,
            reason: format!("kind `{kind_text}` is not one this build knows"),
        };
    };
    if !kind.events().contains(&event) {
        return LineRead::Unknown {
            seq,
            reason: format!("event `{event}` is not one this build knows for `{kind_text}`"),
        };
    }
    match serde_json::from_value::<LogLine>(value) {
        Ok(line) => match line.validate() {
            Ok(()) => LineRead::Entry(line),
            Err(error) => LineRead::Unreadable {
                reason: error.to_string(),
            },
        },
        Err(error) => LineRead::Unreadable {
            reason: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::degradation::{Cause, Degradation};

    fn degradation_line() -> LogLine {
        let d = Degradation {
            cause: "home/seeded-config:gate-policy.json"
                .parse::<Cause>()
                .unwrap(),
            detail: "seeded the shipped gate policy".into(),
            count: 1,
            since: Timestamp::from_millis(5_000).unwrap(),
            cleared_after: None,
        };
        d.to_line(7, Timestamp::from_millis(5_000).unwrap())
            .unwrap()
    }

    #[test]
    fn a_known_line_renders_on_one_line_and_reads_back() {
        let line = degradation_line();
        let text = render_line(&line).unwrap();
        assert!(!text.contains('\n'));
        assert!(
            text.starts_with("{\"schemaVersion\":1,\"seq\":7,\"ts\":\"1970-01-01T00:00:05.000Z\""),
            "{text}"
        );
        assert_eq!(read_line(&text), LineRead::Entry(line));
    }

    #[test]
    fn a_kind_without_events_cannot_be_written_yet() {
        let mut line = degradation_line();
        line.kind = LogKind::Gym;
        line.event = "proposed".into();
        let error = render_line(&line).unwrap_err().to_string();
        assert!(error.contains("not defined yet"), "{error}");
        line.kind = LogKind::Degradation;
        line.event = "exploded".into();
        assert!(
            render_line(&line)
                .unwrap_err()
                .to_string()
                .contains("must be one of: raised, cleared")
        );
    }

    #[test]
    fn the_reader_keeps_what_a_newer_build_wrote_and_reports_what_it_cannot_read() {
        let known = render_line(&degradation_line()).unwrap();
        let newer = known.replace("\"schemaVersion\":1", "\"schemaVersion\":2");
        assert!(matches!(
            read_line(&newer),
            LineRead::Unknown { seq: Some(7), .. }
        ));
        let new_kind = known.replace("\"kind\":\"degradation\"", "\"kind\":\"telepathy\"");
        assert!(matches!(read_line(&new_kind), LineRead::Unknown { .. }));
        let new_event = known.replace("\"event\":\"raised\"", "\"event\":\"dreamt\"");
        assert!(matches!(read_line(&new_event), LineRead::Unknown { .. }));
        let gym = known.replace("\"kind\":\"degradation\"", "\"kind\":\"gym\"");
        assert!(
            matches!(read_line(&gym), LineRead::Unknown { .. }),
            "a kind not yet defined is kept, not refused"
        );

        let torn = &known[..known.len() / 2];
        assert!(matches!(read_line(torn), LineRead::Unreadable { .. }));
        assert!(matches!(
            read_line(&known.replace("\"schemaVersion\":1,", "")),
            LineRead::Unreadable { .. }
        ));
        assert!(matches!(
            read_line(&known.replace("\"schemaVersion\":1", "\"schemaVersion\":0")),
            LineRead::Unreadable { .. }
        ));
        assert!(matches!(
            read_line(&known.replace("\"seq\":7", "\"seq\":0")),
            LineRead::Unreadable { .. }
        ));
        assert!(matches!(
            read_line(&known.replace("\"ts\":\"1970-01-01T00:00:05.000Z\"", "\"ts\":5000")),
            LineRead::Unreadable { .. }
        ));
        let mut value: serde_json::Value = serde_json::from_str(&known).unwrap();
        value.as_object_mut().unwrap().remove("data");
        let LineRead::Unreadable { reason } = read_line(&value.to_string()) else {
            panic!("a line without `data` must not read as an entry");
        };
        assert!(reason.contains("missing field `data`"), "{reason}");
    }

    #[test]
    fn a_damaged_kind_or_event_is_unreadable_not_unknown() {
        // Before review finding F6, a missing kind or event read as "a newer
        // build's kind", so damage was counted as something benign.
        let known: serde_json::Value =
            serde_json::from_str(&render_line(&degradation_line()).unwrap()).unwrap();
        for (field, edit) in [
            ("kind", serde_json::Value::Null),
            ("event", serde_json::Value::from(7)),
        ] {
            let mut value = known.clone();
            value[field] = edit;
            let LineRead::Unreadable { reason } = read_line(&value.to_string()) else {
                panic!("a {field} that is not text must be unreadable");
            };
            assert!(reason.contains("must both be present as text"), "{reason}");
            let mut missing = known.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                matches!(read_line(&missing.to_string()), LineRead::Unreadable { .. }),
                "{field}"
            );
        }
        let twice = render_line(&degradation_line()).unwrap().replacen(
            "\"seq\":7,",
            "\"seq\":7,\"seq\":8,",
            1,
        );
        let LineRead::Unreadable { reason } = read_line(&twice) else {
            panic!("a line naming a key twice must be unreadable: {twice}");
        };
        assert!(reason.contains("`seq` appears twice"), "{reason}");
    }

    #[test]
    fn a_source_is_one_word() {
        let mut line = degradation_line();
        line.source = Some("agent.mason".into());
        assert!(render_line(&line).is_ok());
        line.source = Some("two words".into());
        assert!(
            render_line(&line)
                .unwrap_err()
                .to_string()
                .contains("source")
        );
    }

    #[test]
    fn every_kind_is_listed_once() {
        let mut kinds = LogKind::ALL.to_vec();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), LogKind::ALL.len());
    }
}

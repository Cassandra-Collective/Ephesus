//! Two restart records R1 types (SDD-RUST §4.2; ADR-0027): `triggers.json`,
//! the one piece of scheduler state a restart cannot re-derive, and
//! `breaker-stops.json`, the durable rung-3 stops. The other three restart
//! records (activations, gates, drafts) are typed by the phases that build
//! their subsystems (Architect, 2026-10-20).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::names::AgentId;
use crate::time::Timestamp;

/// `triggers.json`: trigger id → when it last fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Triggers {
    /// [`Triggers::VERSION`].
    pub schema_version: u32,
    /// Keyed by trigger id (`standup`, `<instance>/<trigger>`).
    pub last_fired: BTreeMap<String, Timestamp>,
}

impl Triggers {
    /// No trigger has fired.
    pub fn empty() -> Triggers {
        Triggers {
            schema_version: Self::VERSION,
            last_fired: BTreeMap::new(),
        }
    }
}

impl Versioned for Triggers {
    const FILE: &'static str = "triggers.json";
    const VERSION: u32 = 2;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        for id in self.last_fired.keys() {
            let ok =
                !id.is_empty() && id.chars().count() <= 128 && !id.chars().any(char::is_control);
            if !ok {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("lastFired.{id:?}"),
                    "a trigger id is 1 to 128 printable characters",
                ));
            }
        }
        Ok(())
    }
}

/// What tripped a breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Signal {
    /// The same call over and over.
    Repetition,
    /// Errors faster than the threshold.
    ErrorRate,
    /// A conversation past the hop cap.
    HopCap,
    /// Tokens spent faster than the budget allows.
    BurnRate,
}

/// A value in a stop's detail: text or a number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    /// Text.
    Text(String),
    /// A number.
    Number(serde_json::Number),
}

/// One durable stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Stop {
    /// When it was stopped.
    pub at: Timestamp,
    /// What tripped it: at least one signal.
    pub signals: BTreeSet<Signal>,
    /// What each signal measured.
    pub detail: Vec<BTreeMap<String, Scalar>>,
}

/// `breaker-stops.json`: agent → its stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BreakerStops {
    /// [`BreakerStops::VERSION`].
    pub schema_version: u32,
    /// One stop per agent.
    pub stops: BTreeMap<AgentId, Stop>,
}

impl BreakerStops {
    /// No agent is stopped.
    pub fn empty() -> BreakerStops {
        BreakerStops {
            schema_version: Self::VERSION,
            stops: BTreeMap::new(),
        }
    }
}

impl Versioned for BreakerStops {
    const FILE: &'static str = "breaker-stops.json";
    const VERSION: u32 = 2;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        for (agent, stop) in &self.stops {
            if stop.signals.is_empty() {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("stops.{agent}.signals"),
                    "a stop names at least one signal that tripped it",
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

    #[test]
    fn triggers_round_trip_and_refuse_a_bad_id() {
        let text = r#"{"schemaVersion":2,"lastFired":{"standup":"2026-10-19T08:00:00.000Z","crew@repo-x/ci":"2026-10-19T08:05:00.000Z"}}"#;
        let triggers = parse::<Triggers>(text).unwrap();
        assert_eq!(triggers.last_fired.len(), 2);
        assert_eq!(
            parse::<Triggers>(&render(&triggers).unwrap()).unwrap(),
            triggers
        );
        let long = format!(
            r#"{{"schemaVersion":2,"lastFired":{{"{}":"2026-10-19T08:00:00.000Z"}}}}"#,
            "x".repeat(129)
        );
        assert!(
            parse::<Triggers>(&long)
                .unwrap_err()
                .to_string()
                .contains("1 to 128")
        );
        assert!(
            parse::<Triggers>(r#"{"schemaVersion":2,"lastFired":{"":"2026-10-19T08:00:00.000Z"}}"#)
                .is_err()
        );
        assert!(
            parse::<Triggers>(r#"{"schemaVersion":2,"lastFired":{"standup":1760877296789}}"#)
                .is_err(),
            "epoch ms is the old format"
        );
    }

    #[test]
    fn a_stop_names_its_signals_and_keeps_its_detail() {
        let text = r#"{"schemaVersion":2,"stops":{"agent.mason":{"at":"2026-10-19T08:00:00.000Z",
            "signals":["burn-rate","repetition"],"detail":[{"signal":"burn-rate","tokensPerMin":12000}]}}}"#;
        let stops = parse::<BreakerStops>(text).unwrap();
        let stop = &stops.stops[&"agent.mason".parse::<AgentId>().unwrap()];
        assert!(stop.signals.contains(&Signal::BurnRate));
        assert_eq!(stop.detail[0]["tokensPerMin"], Scalar::Number(12000.into()));
        assert_eq!(
            parse::<BreakerStops>(&render(&stops).unwrap()).unwrap(),
            stops
        );
        let none = r#"{"schemaVersion":2,"stops":{"agent.mason":{"at":"2026-10-19T08:00:00.000Z","signals":[],"detail":[]}}}"#;
        assert!(
            parse::<BreakerStops>(none)
                .unwrap_err()
                .to_string()
                .contains("at least one signal")
        );
        assert!(parse::<BreakerStops>(r#"{"schemaVersion":2,"stops":{"mason":{"at":"2026-10-19T08:00:00.000Z","signals":["hop-cap"]}}}"#).is_err());
    }

    #[test]
    fn the_empty_documents_are_valid_and_current() {
        let triggers = Triggers::empty();
        assert_eq!(triggers.schema_version, Triggers::VERSION);
        assert_eq!(
            parse::<Triggers>(&render(&triggers).unwrap()).unwrap(),
            triggers
        );
        let stops = BreakerStops::empty();
        assert_eq!(stops.schema_version, BreakerStops::VERSION);
        assert_eq!(
            parse::<BreakerStops>(&render(&stops).unwrap()).unwrap(),
            stops
        );
    }
}

//! `home.json`: the marker of a home this build made (SDD-RUST §2, §4.4;
//! Architect, 2026-10-20).
//!
//! Contract: a directory is this build's home only if it holds a valid
//! `home.json`. The TypeScript build keeps running from `~/.ephesus` until R14,
//! so an existing directory without the marker is never written to (`eph-fs`
//! holds that rule; this type is the file it reads and writes).

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::time::Timestamp;

/// `home.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HomeMarker {
    /// [`HomeMarker::VERSION`].
    pub schema_version: u32,
    /// When this build created the home.
    pub created_at: Timestamp,
}

impl HomeMarker {
    /// The marker for a home created at `at`.
    pub fn new(at: Timestamp) -> HomeMarker {
        HomeMarker {
            schema_version: Self::VERSION,
            created_at: at,
        }
    }
}

impl Versioned for HomeMarker {
    const FILE: &'static str = "home.json";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{parse, render};

    #[test]
    fn a_marker_round_trips_and_carries_nothing_else() {
        let marker = HomeMarker::new(Timestamp::from_millis(1_000).unwrap());
        let text = render(&marker).unwrap();
        assert_eq!(
            text,
            "{\n  \"schemaVersion\": 1,\n  \"createdAt\": \"1970-01-01T00:00:01.000Z\"\n}\n"
        );
        assert_eq!(parse::<HomeMarker>(&text).unwrap(), marker);
        let extra = text.replace("\"createdAt\"", "\"build\": \"rust\",\n  \"createdAt\"");
        assert!(
            parse::<HomeMarker>(&extra)
                .unwrap_err()
                .to_string()
                .contains("build")
        );
        let wrong = HomeMarker {
            schema_version: 2,
            ..marker
        };
        assert!(render(&wrong).is_err());
    }
}

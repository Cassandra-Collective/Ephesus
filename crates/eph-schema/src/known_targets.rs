//! `known-targets.json`: targets the profile panel offers, most recent first
//! (SDD-RUST §4.1). A convenience, never a restore list: choosing one still goes
//! through preview and activation.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::names::{Slug, check_len};
use crate::time::Timestamp;

/// How many targets [`KnownTargets::remember`] keeps.
pub const KNOWN_TARGETS_LIMIT: usize = 50;

/// What a profile targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    /// A repository.
    Repo,
    /// An application.
    App,
}

/// A target: its kind, its id, and where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Target {
    /// Its kind.
    pub kind: TargetKind,
    /// Its id, a lowercase name of at most 48 characters.
    pub id: Slug,
    /// Its path on this machine.
    pub path: String,
}

/// One remembered activation target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct KnownTarget {
    /// The profile it was used with.
    pub profile: Slug,
    /// The target.
    pub target: Target,
    /// When it was last used.
    pub last_used_at: Timestamp,
}

/// `known-targets.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct KnownTargets {
    /// [`KnownTargets::VERSION`].
    pub schema_version: u32,
    /// Most recent first.
    pub targets: Vec<KnownTarget>,
}

impl KnownTargets {
    /// No targets.
    pub fn empty() -> KnownTargets {
        KnownTargets {
            schema_version: Self::VERSION,
            targets: Vec::new(),
        }
    }

    /// Records `entry` as the most recent use, replacing an earlier use of the
    /// same profile and target, and keeps at most [`KNOWN_TARGETS_LIMIT`].
    pub fn remember(&mut self, entry: KnownTarget) {
        self.targets.retain(|known| {
            !(known.profile == entry.profile
                && known.target.kind == entry.target.kind
                && known.target.id == entry.target.id)
        });
        self.targets.insert(0, entry);
        self.targets.truncate(KNOWN_TARGETS_LIMIT);
    }
}

impl Versioned for KnownTargets {
    const FILE: &'static str = "known-targets.json";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if self.targets.len() > 200 {
            return Err(SchemaError::invalid(
                Self::FILE,
                "targets",
                "must hold at most 200 targets",
            ));
        }
        let mut seen = BTreeSet::new();
        for (i, known) in self.targets.iter().enumerate() {
            if known.target.id.as_str().chars().count() > 48 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("targets[{i}].target.id"),
                    "must be at most 48 characters",
                ));
            }
            check_len(
                Self::FILE,
                &format!("targets[{i}].target.path"),
                &known.target.path,
                1,
                4096,
            )?;
            if !seen.insert((
                known.profile.clone(),
                known.target.kind,
                known.target.id.clone(),
            )) {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    format!("targets[{i}]"),
                    "repeats an earlier profile and target; a target is remembered once",
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

    fn known(profile: &str, id: &str, millis: i64) -> KnownTarget {
        KnownTarget {
            profile: profile.parse().unwrap(),
            target: Target {
                kind: TargetKind::Repo,
                id: id.parse().unwrap(),
                path: format!("C:/work/{id}"),
            },
            last_used_at: Timestamp::from_millis(millis).unwrap(),
        }
    }

    #[test]
    fn remember_puts_the_latest_first_replaces_a_repeat_and_trims() {
        let mut targets = KnownTargets::empty();
        targets.remember(known("crew", "a", 1));
        targets.remember(known("crew", "b", 2));
        targets.remember(known("crew", "a", 3));
        let ids: Vec<&str> = targets
            .targets
            .iter()
            .map(|k| k.target.id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b"]);
        for i in 0..60 {
            targets.remember(known("crew", &format!("t{i}"), 10 + i));
        }
        assert_eq!(targets.targets.len(), KNOWN_TARGETS_LIMIT);
        assert_eq!(targets.targets[0].target.id.as_str(), "t59");
        assert_eq!(
            parse::<KnownTargets>(&render(&targets).unwrap()).unwrap(),
            targets
        );
    }

    #[test]
    fn a_repeat_an_overlong_id_or_an_empty_path_is_refused() {
        let mut targets = KnownTargets::empty();
        targets.targets = vec![known("crew", "a", 1), known("crew", "a", 2)];
        assert!(
            render(&targets)
                .unwrap_err()
                .to_string()
                .contains("remembered once")
        );
        targets.targets = vec![known("crew", &"x".repeat(49), 1)];
        assert!(
            render(&targets)
                .unwrap_err()
                .to_string()
                .contains("at most 48")
        );
        let mut empty_path = known("crew", "a", 1);
        empty_path.target.path = String::new();
        targets.targets = vec![empty_path];
        assert!(
            render(&targets)
                .unwrap_err()
                .to_string()
                .contains("target.path")
        );
    }

    #[test]
    fn a_list_past_its_cap_is_refused() {
        let targets = KnownTargets {
            schema_version: 1,
            targets: (0..201)
                .map(|i| known("crew", &format!("t{i}"), i64::from(i)))
                .collect(),
        };
        let error = render(&targets).unwrap_err().to_string();
        assert!(error.contains("must hold at most 200 targets"), "{error}");
        let at_cap = KnownTargets {
            schema_version: 1,
            targets: (0..200)
                .map(|i| known("crew", &format!("t{i}"), i64::from(i)))
                .collect(),
        };
        assert!(render(&at_cap).is_ok());
        assert!(parse::<KnownTargets>(&render(&KnownTargets::empty()).unwrap()).is_ok());
    }
}

//! `authority.json`: what Artemis may decide on the Architect's behalf
//! (SDD-RUST §4.1; FR-5.5).
//!
//! Contract: absent or damaged means [`Authority::none`], so everything
//! escalates. A spend grant carries its ceiling, and only a spend grant may.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::names::Slug;

/// What a grant covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthorityClass {
    /// Routing mail between agents.
    Route,
    /// Creating and updating tasks.
    Task,
    /// Settling gates.
    Gate,
    /// Approving spend.
    Spend,
    /// Deciding memos.
    Memo,
}

/// One grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Grant {
    /// What it covers.
    pub class: AuthorityClass,
    /// The domains it covers: `*` or lowercase names, 1 to 32 of them.
    pub domains: Vec<String>,
    /// A spend grant's ceiling, in tokens: required on spend, refused elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_spend_tokens: Option<u64>,
}

/// `authority.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Authority {
    /// [`Authority::VERSION`].
    pub schema_version: u32,
    /// The shipped version this file was seeded from (SDD-RUST §4.0 rule 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeded_from: Option<u32>,
    /// At most 64 grants.
    pub grants: Vec<Grant>,
}

impl Authority {
    /// No delegated authority: everything escalates to the Architect.
    pub fn none() -> Authority {
        Authority {
            schema_version: Self::VERSION,
            seeded_from: None,
            grants: Vec::new(),
        }
    }
}

impl Versioned for Authority {
    const FILE: &'static str = "authority.json";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if self.grants.len() > 64 {
            return Err(SchemaError::invalid(
                Self::FILE,
                "grants",
                "must hold at most 64 grants",
            ));
        }
        let mut seen = BTreeSet::new();
        for (i, grant) in self.grants.iter().enumerate() {
            let field = |name: &str| format!("grants[{i}].{name}");
            if grant.domains.is_empty() || grant.domains.len() > 32 {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("domains"),
                    "must name 1 to 32 domains",
                ));
            }
            for domain in &grant.domains {
                if domain != "*" && domain.parse::<Slug>().is_err() {
                    return Err(SchemaError::invalid(
                        Self::FILE,
                        field("domains"),
                        format!("`{domain}` is neither `*` nor a lowercase name like test-code"),
                    ));
                }
            }
            let spend = grant.class == AuthorityClass::Spend;
            match grant.max_spend_tokens {
                None if spend => {
                    return Err(SchemaError::invalid(
                        Self::FILE,
                        field("maxSpendTokens"),
                        "a spend grant states its ceiling",
                    ));
                }
                Some(_) if !spend => {
                    return Err(SchemaError::invalid(
                        Self::FILE,
                        field("maxSpendTokens"),
                        "only a spend grant carries a ceiling",
                    ));
                }
                Some(t) if t == 0 || t > 1_000_000_000 => {
                    return Err(SchemaError::invalid(
                        Self::FILE,
                        field("maxSpendTokens"),
                        "must be 1 to 1000000000",
                    ));
                }
                _ => {}
            }
            let mut domains = grant.domains.clone();
            domains.sort();
            if !seen.insert((grant.class, domains)) {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("class"),
                    "duplicates an earlier grant of the same class over the same domains",
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

    const GRANTS: &str = r#"{"schemaVersion":1,"seededFrom":1,"grants":[
        {"class":"route","domains":["*"]},
        {"class":"memo","domains":["test-code","docs"]},
        {"class":"spend","domains":["*"],"maxSpendTokens":50000}]}"#;

    #[test]
    fn grants_parse_and_round_trip() {
        let authority = parse::<Authority>(GRANTS).unwrap();
        assert_eq!(authority.grants.len(), 3);
        assert_eq!(
            parse::<Authority>(&render(&authority).unwrap()).unwrap(),
            authority
        );
        assert!(Authority::none().grants.is_empty());
    }

    #[test]
    fn each_refinement_refuses_naming_the_grant() {
        let cases = [
            (
                r#"{"class":"spend","domains":["*"]}"#,
                "a spend grant states its ceiling",
            ),
            (
                r#"{"class":"memo","domains":["*"],"maxSpendTokens":5}"#,
                "only a spend grant carries a ceiling",
            ),
            (
                r#"{"class":"spend","domains":["*"],"maxSpendTokens":0}"#,
                "must be 1 to 1000000000",
            ),
            (r#"{"class":"route","domains":[]}"#, "1 to 32 domains"),
            (
                r#"{"class":"route","domains":["Test Code"]}"#,
                "neither `*` nor",
            ),
        ];
        for (grant, needle) in cases {
            let text = format!(r#"{{"schemaVersion":1,"grants":[{grant}]}}"#);
            let error = parse::<Authority>(&text).unwrap_err().to_string();
            assert!(
                error.contains(needle) && error.contains("grants[0]"),
                "{needle}: {error}"
            );
        }
    }

    #[test]
    fn the_same_class_over_the_same_domains_in_any_order_is_a_duplicate() {
        let text = r#"{"schemaVersion":1,"grants":[{"class":"memo","domains":["a","b"]},{"class":"memo","domains":["b","a"]}]}"#;
        let error = parse::<Authority>(text).unwrap_err().to_string();
        assert!(
            error.contains("grants[1].class") && error.contains("duplicates"),
            "{error}"
        );
        let distinct = r#"{"schemaVersion":1,"grants":[{"class":"memo","domains":["a"]},{"class":"task","domains":["a"]}]}"#;
        assert!(parse::<Authority>(distinct).is_ok());
    }

    #[test]
    fn more_than_sixty_four_grants_is_refused() {
        let grant = r#"{"class":"route","domains":["x"]}"#;
        let many: Vec<String> = (0..65)
            .map(|i| grant.replace("\"x\"", &format!("\"d{i}\"")))
            .collect();
        let text = format!(r#"{{"schemaVersion":1,"grants":[{}]}}"#, many.join(","));
        assert!(
            parse::<Authority>(&text)
                .unwrap_err()
                .to_string()
                .contains("at most 64")
        );
    }

    /// Round 1 (2026-10-20): every cap was tested past its limit and none at it.
    #[test]
    fn each_cap_admits_its_limit_and_refuses_one_more() {
        let table = |grants: Vec<String>| {
            format!("{{\"schemaVersion\":1,\"grants\":[{}]}}", grants.join(","))
        };
        let routes = |n: usize| -> Vec<String> {
            (0..n)
                .map(|i| format!("{{\"class\":\"route\",\"domains\":[\"d{i}\"]}}"))
                .collect()
        };
        assert!(parse::<Authority>(&table(routes(64))).is_ok(), "64 grants");
        assert!(parse::<Authority>(&table(routes(65))).is_err(), "65 grants");
        let domains = |n: usize| {
            let names: Vec<String> = (0..n).map(|i| format!("\"d{i}\"")).collect();
            table(vec![format!(
                "{{\"class\":\"route\",\"domains\":[{}]}}",
                names.join(",")
            )])
        };
        assert!(parse::<Authority>(&domains(32)).is_ok(), "32 domains");
        let error = parse::<Authority>(&domains(33)).unwrap_err().to_string();
        assert!(error.contains("grants[0].domains"), "33 domains: {error}");
        let spend = |tokens: u64| {
            table(vec![format!(
                "{{\"class\":\"spend\",\"domains\":[\"*\"],\"maxSpendTokens\":{tokens}}}"
            )])
        };
        assert!(
            parse::<Authority>(&spend(1_000_000_000)).is_ok(),
            "the ceiling's ceiling"
        );
        assert!(
            parse::<Authority>(&spend(1_000_000_001)).is_err(),
            "one over"
        );
    }
}

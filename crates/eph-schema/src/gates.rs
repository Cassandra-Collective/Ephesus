//! `gate-policy.json`: the Watch's gate policy (SDD-RUST §4.1; FR-11.1, FR-1.9,
//! FR-11.7).
//!
//! Contract: absent or damaged means [`GatePolicy::deny_all`], which can only
//! loosen, never tighten, when the Architect writes a real one. Rules are keyed
//! by gate kind, so "at most one rule per kind" holds by construction.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;

/// What a gate guards (FR-11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateKind {
    /// Deleting or overwriting something that cannot be restored.
    Destructive,
    /// Spending above a threshold.
    Spend,
    /// Changing the scope of a task or a profile.
    ScopeChange,
    /// Touching something production-facing.
    ProdFacing,
    /// Public communication from the company (FR-9.3).
    Outbound,
    /// An engine's own permission prompt.
    ToolPermission,
    /// Anything an agent says needs a human.
    NeedsHuman,
}

/// How much an agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Autonomy {
    /// Every gated action waits for the Architect.
    Manual,
    /// Routine actions proceed; gated ones wait.
    Supervised,
    /// Gated actions proceed within budget.
    Autonomous,
}

/// Where an approval may come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// The window or the control socket.
    Local,
    /// The Herald, with repeat-back where required (FR-8.4).
    Voice,
    /// The Harbor's bridge (FR-10.2).
    Remote,
}

/// One kind's rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GateRule {
    /// The autonomy at which this kind proceeds without asking.
    pub autonomy: Autonomy,
    /// A spend gate's threshold, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_spend_tokens: Option<u64>,
    /// Where an approval may come from; absent means local only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<BTreeSet<Channel>>,
    /// Whether a voice approval needs repeat-back (FR-8.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_repeat_back: Option<bool>,
}

/// `gate-policy.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GatePolicy {
    /// [`GatePolicy::VERSION`].
    pub schema_version: u32,
    /// The shipped policy version this file was seeded from (SDD-RUST §4.0
    /// rule 8); absent on a file the Architect wrote from scratch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeded_from: Option<u32>,
    /// The company-wide autonomy ceiling.
    pub autonomy: Autonomy,
    /// The company-wide daily token ceiling (FR-1.9); absent means none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_daily_tokens: Option<u64>,
    /// One rule per gate kind.
    pub rules: BTreeMap<GateKind, GateRule>,
}

const MAX_TOKENS: u64 = 1_000_000_000;

impl GatePolicy {
    /// The policy in force when the file is absent or damaged: everything waits
    /// for the Architect.
    pub fn deny_all() -> GatePolicy {
        GatePolicy {
            schema_version: Self::VERSION,
            seeded_from: None,
            autonomy: Autonomy::Manual,
            max_daily_tokens: None,
            rules: BTreeMap::new(),
        }
    }
}

impl Versioned for GatePolicy {
    const FILE: &'static str = "gate-policy.json";
    const VERSION: u32 = 2;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        if let Some(tokens) = self.max_daily_tokens
            && !(1..=MAX_TOKENS).contains(&tokens)
        {
            return Err(SchemaError::invalid(
                Self::FILE,
                "maxDailyTokens",
                "must be 1 to 1000000000; remove the field for no ceiling",
            ));
        }
        for (kind, rule) in &self.rules {
            let field = |name: &str| {
                let kind = serde_json::to_value(kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                format!("rules.{kind}.{name}")
            };
            if rule.max_spend_tokens.is_some_and(|t| t > MAX_TOKENS) {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("maxSpendTokens"),
                    "must be at most 1000000000",
                ));
            }
            if rule.channels.as_ref().is_some_and(BTreeSet::is_empty) {
                return Err(SchemaError::invalid(
                    Self::FILE,
                    field("channels"),
                    "must name at least one channel; remove the field for local only",
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
    fn deny_all_waits_for_the_architect_and_round_trips() {
        let policy = GatePolicy::deny_all();
        assert_eq!(policy.autonomy, Autonomy::Manual);
        assert!(policy.rules.is_empty());
        assert_eq!(
            parse::<GatePolicy>(&render(&policy).unwrap()).unwrap(),
            policy
        );
    }

    #[test]
    fn rules_are_keyed_by_kind_so_a_kind_has_one_rule() {
        let text = r#"{"schemaVersion":2,"seededFrom":1,"autonomy":"autonomous","maxDailyTokens":200000,
            "rules":{"spend":{"autonomy":"supervised","maxSpendTokens":200000},
                     "outbound":{"autonomy":"supervised","channels":["local","remote"],"requireRepeatBack":true},
                     "needs-human":{"autonomy":"manual"}}}"#;
        let policy = parse::<GatePolicy>(text).unwrap();
        assert_eq!(policy.rules.len(), 3);
        assert_eq!(
            policy.rules[&GateKind::NeedsHuman].autonomy,
            Autonomy::Manual
        );
        assert_eq!(
            parse::<GatePolicy>(&render(&policy).unwrap()).unwrap(),
            policy
        );
        assert!(
            parse::<GatePolicy>(
                r#"{"schemaVersion":2,"autonomy":"manual","rules":{"bogus":{"autonomy":"manual"}}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn a_kind_written_twice_is_refused_not_silently_overruled() {
        // serde_json keeps the last of two equal keys, so this read as
        // `autonomous`: a second rule loosened the first without a word (review
        // finding F1, 2026-10-20). Today's build refused two rules for one kind.
        let twice = r#"{"schemaVersion":2,"autonomy":"manual","rules":{
            "destructive":{"autonomy":"supervised"},
            "destructive":{"autonomy":"autonomous"}}}"#;
        let error = parse::<GatePolicy>(twice).unwrap_err().to_string();
        assert!(error.contains("`destructive` appears twice"), "{error}");
    }

    #[test]
    fn each_rule_refuses_naming_the_field() {
        let cases = [
            (
                r#"{"schemaVersion":2,"autonomy":"manual","maxDailyTokens":0,"rules":{}}"#,
                "maxDailyTokens",
            ),
            (
                r#"{"schemaVersion":2,"autonomy":"manual","rules":{"spend":{"autonomy":"manual","maxSpendTokens":1000000001}}}"#,
                "rules.spend.maxSpendTokens",
            ),
            (
                r#"{"schemaVersion":2,"autonomy":"manual","rules":{"outbound":{"autonomy":"manual","channels":[]}}}"#,
                "rules.outbound.channels",
            ),
        ];
        for (text, field) in cases {
            let error = parse::<GatePolicy>(text).unwrap_err().to_string();
            assert!(error.contains(field), "{field}: {error}");
        }
        assert!(
            parse::<GatePolicy>(r#"{"schemaVersion":2,"rules":{}}"#).is_err(),
            "autonomy is required"
        );
    }

    #[test]
    fn autonomy_is_ordered_from_manual_to_autonomous() {
        assert!(Autonomy::Manual < Autonomy::Supervised);
        assert!(Autonomy::Supervised < Autonomy::Autonomous);
    }

    /// Round 1 (2026-10-20): the spend ceiling was tested one over, never at.
    #[test]
    fn a_rule_may_spend_up_to_the_ceiling() {
        let at = r#"{"schemaVersion":2,"autonomy":"manual","rules":{"spend":{"autonomy":"manual","maxSpendTokens":1000000000}}}"#;
        assert!(parse::<GatePolicy>(at).is_ok());
    }
}

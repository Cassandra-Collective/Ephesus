//! The names Ephesus files use: agent ids, addresses, slugs and environment
//! variable names, each a type that cannot hold a value its grammar refuses.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The ids the harness writes mail under, which no hire may take: a hire that
/// took one could forge a router refusal or a ledger reply in the harness's
/// name.
pub const RESERVED_AGENT_IDS: &[&str] = &[
    "agent.hermes",
    "agent.ledger",
    "agent.library",
    "agent.closing",
    "agent.odeon",
    "agent.harbor",
    "agent.profiles",
];

/// A text that breaks a name's grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadName {
    /// What the name is (`agent id`).
    pub what: &'static str,
    /// What is allowed.
    pub rule: &'static str,
    /// The text refused.
    pub text: String,
}

impl fmt::Display for BadName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a valid {}: {}",
            self.text, self.what, self.rule
        )
    }
}

fn is_slug_body(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident, $what:literal, $rule:literal, $check:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// The name as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = BadName;

            fn from_str(text: &str) -> Result<$name, BadName> {
                let check: fn(&str) -> bool = $check;
                if check(text) {
                    Ok($name(text.to_string()))
                } else {
                    Err(BadName { what: $what, rule: $rule, text: text.to_string() })
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(|error: BadName| serde::de::Error::custom(error))
            }
        }
    };
}

name_type!(
    /// An agent's id: `agent.` and a lowercase slug, at most 64 characters.
    AgentId,
    "agent id",
    "`agent.` followed by lowercase letters, digits and dashes, at most 64 characters",
    |text| text.len() <= 64 && text.strip_prefix("agent.").is_some_and(is_slug_body)
);

name_type!(
    /// A lowercase name: profiles, hire templates, trigger ids, target ids.
    Slug,
    "name",
    "lowercase letters, digits and dashes, starting with a letter or digit, at most 64 characters",
    |text| text.len() <= 64 && is_slug_body(text)
);

name_type!(
    /// An environment variable's name: `GH_TOKEN`.
    EnvName,
    "environment variable name",
    "an uppercase letter, then uppercase letters, digits and underscores, at most 64 characters",
    |text| {
        let mut bytes = text.bytes();
        text.len() <= 64
            && matches!(bytes.next(), Some(b'A'..=b'Z'))
            && bytes.all(|b| matches!(b, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
    }
);

impl AgentId {
    /// Whether the harness writes mail under this id, so no hire may take it.
    pub fn is_reserved(&self) -> bool {
        RESERVED_AGENT_IDS.contains(&self.as_str())
    }
}

/// Where a message goes: one agent, every agent, or the Architect.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Address {
    /// One agent.
    Agent(AgentId),
    /// Every agent on the roster.
    Broadcast,
    /// The Architect, through Artemis (FR-3.7).
    Human,
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Address::Agent(id) => f.write_str(id.as_str()),
            Address::Broadcast => f.write_str("broadcast"),
            Address::Human => f.write_str("human"),
        }
    }
}

impl FromStr for Address {
    type Err = BadName;

    fn from_str(text: &str) -> Result<Address, BadName> {
        match text {
            "broadcast" => Ok(Address::Broadcast),
            "human" => Ok(Address::Human),
            other => other.parse().map(Address::Agent).map_err(|_| BadName {
                what: "address",
                rule: "an agent id, `broadcast` or `human`",
                text: text.to_string(),
            }),
        }
    }
}

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Address, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|error: BadName| serde::de::Error::custom(error))
    }
}

/// Checks a free-text field's length in characters, naming the rule.
pub fn check_len(
    file: &'static str,
    field: &str,
    text: &str,
    min: usize,
    max: usize,
) -> Result<(), crate::SchemaError> {
    let n = text.chars().count();
    if n < min || n > max {
        return Err(crate::SchemaError::invalid(
            file,
            field,
            format!("must be {min} to {max} characters; it is {n}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_id_is_agent_dot_slug() {
        for good in ["agent.mason", "agent.on-call-2", "agent.0"] {
            assert!(good.parse::<AgentId>().is_ok(), "{good}");
        }
        for bad in [
            "mason",
            "agent.",
            "agent.Mason",
            "agent.-x",
            "agent.a_b",
            "agents.x",
            &format!("agent.{}", "a".repeat(60)),
        ] {
            let error = bad.parse::<AgentId>().unwrap_err();
            assert!(error.to_string().contains("`agent.` followed by"), "{bad}");
        }
    }

    #[test]
    fn the_harness_ids_are_reserved_and_hires_are_not() {
        assert!("agent.ledger".parse::<AgentId>().unwrap().is_reserved());
        assert!(!"agent.mason".parse::<AgentId>().unwrap().is_reserved());
        assert_eq!(RESERVED_AGENT_IDS.len(), 7);
    }

    #[test]
    fn slugs_and_env_names_have_their_grammars() {
        assert!("skeleton-crew".parse::<Slug>().is_ok());
        for bad in ["", "-x", "Upper", "a b", "a_b", &"a".repeat(65)] {
            assert!(bad.parse::<Slug>().is_err(), "{bad}");
        }
        assert!("GH_TOKEN".parse::<EnvName>().is_ok());
        for bad in ["", "gh_token", "1TOKEN", "GH-TOKEN", "_X"] {
            assert!(bad.parse::<EnvName>().is_err(), "{bad}");
        }
    }

    #[test]
    fn an_address_is_an_agent_broadcast_or_the_architect() {
        assert_eq!("human".parse::<Address>(), Ok(Address::Human));
        assert_eq!("broadcast".parse::<Address>(), Ok(Address::Broadcast));
        assert_eq!("agent.x".parse::<Address>().unwrap().to_string(), "agent.x");
        let error = "everyone".parse::<Address>().unwrap_err();
        assert!(error.to_string().contains("`broadcast` or `human`"));
    }

    #[test]
    fn a_length_check_counts_characters_and_says_the_bounds() {
        assert!(check_len("f", "x", "ab", 1, 2).is_ok());
        assert!(check_len("f", "x", "üü", 1, 2).is_ok());
        let error = check_len("f", "x", "abc", 1, 2).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be 1 to 2 characters; it is 3")
        );
        assert!(check_len("f", "x", "", 1, 2).is_err());
    }

    #[test]
    fn every_address_prints_as_it_parses() {
        for text in ["agent.mason", "broadcast", "human"] {
            assert_eq!(text.parse::<Address>().unwrap().to_string(), text);
        }
    }
}

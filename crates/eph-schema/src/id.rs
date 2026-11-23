//! One id grammar (SDD-RUST §4.0 rule 3): `<kind>-<compact UTC>-<8 hex>`, for
//! example `t-20261019T123456789Z-1a2b3c4d`.
//!
//! Contract: an id sorts by the time it was minted, is safe as a file name on
//! every platform, and is simple enough for an agent to write without a library
//! (an agent mints its own message ids). Parsing is strict: the kind prefix, the
//! compact time and exactly eight lowercase hex digits.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::time::Timestamp;

/// Eight random lowercase hex digits, for minting an id. Side effect: reads the
/// operating system's randomness (a uuid v4).
pub fn random_suffix() -> String {
    let bytes = uuid::Uuid::new_v4();
    let hex = bytes.simple().to_string();
    hex[..8].to_string()
}

fn parse_id(prefix: &str, text: &str) -> Option<(Timestamp, String)> {
    let rest = text.strip_prefix(prefix)?.strip_prefix('-')?;
    let (time, suffix) = rest.split_once('-')?;
    let suffix_ok = suffix.len() == 8
        && suffix
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !suffix_ok {
        return None;
    }
    let at = Timestamp::parse_compact(time).ok()?;
    Some((at, suffix.to_string()))
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            // Ordering compares the text first, and the text sorts by time
            // because the compact time is fixed-width.
            text: String,
            at: Timestamp,
        }

        impl $name {
            /// The prefix every id of this kind starts with.
            pub const PREFIX: &'static str = $prefix;

            /// Mints an id at `at` with `suffix`, which must be eight lowercase
            /// hex digits ([`random_suffix`] makes one). `None` if it is not.
            pub fn mint(at: Timestamp, suffix: &str) -> Option<$name> {
                format!("{}-{}-{}", $prefix, at.compact(), suffix).parse().ok()
            }

            /// Mints a fresh id now. Side effects: reads the system clock and
            /// the operating system's randomness.
            pub fn new_now() -> $name {
                let at = Timestamp::now();
                let text = format!("{}-{}-{}", $prefix, at.compact(), random_suffix());
                $name { text, at }
            }

            /// When it was minted.
            pub fn minted_at(&self) -> Timestamp {
                self.at
            }

            /// The id as text.
            pub fn as_str(&self) -> &str {
                &self.text
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.text)
            }
        }

        impl FromStr for $name {
            type Err = BadId;

            fn from_str(text: &str) -> Result<$name, BadId> {
                match parse_id($prefix, text) {
                    Some((at, _)) => Ok($name { text: text.to_string(), at }),
                    None => Err(BadId { prefix: $prefix, text: text.to_string() }),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.text)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(|error: BadId| serde::de::Error::custom(error))
            }
        }
    };
}

id_type!(
    /// A Hermes message's id (`msg-…`).
    MsgId,
    "msg"
);
id_type!(
    /// A task's id (`t-…`).
    TaskId,
    "t"
);
id_type!(
    /// A gate's id (`g-…`).
    GateId,
    "g"
);
id_type!(
    /// A decision memo's id (`m-…`).
    MemoId,
    "m"
);
id_type!(
    /// A meeting's id (`mt-…`).
    MeetingId,
    "mt"
);
id_type!(
    /// A standup brief's id (`brief-…`).
    BriefId,
    "brief"
);
id_type!(
    /// A mission activation's id (`act-…`).
    ActivationId,
    "act"
);

/// A text that is not an id of the expected kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadId {
    /// The kind's prefix.
    pub prefix: &'static str,
    /// The text refused.
    pub text: String,
}

impl fmt::Display for BadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a `{}` id; an id is `{}-<compact UTC>-<8 hex>`, like {}-20261019T123456789Z-1a2b3c4d",
            self.text, self.prefix, self.prefix, self.prefix
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Timestamp {
        Timestamp::from_millis(1_760_877_296_789).unwrap()
    }

    #[test]
    fn an_id_is_kind_compact_time_and_eight_hex() {
        let id = TaskId::mint(at(), "1a2b3c4d").unwrap();
        assert_eq!(id.as_str(), "t-20251019T123456789Z-1a2b3c4d");
        assert_eq!(id.minted_at(), at());
        assert_eq!(id.to_string().parse::<TaskId>(), Ok(id));
    }

    #[test]
    fn ids_sort_by_the_time_they_were_minted() {
        let early = MsgId::mint(at(), "ffffffff").unwrap();
        let late = MsgId::mint(
            Timestamp::from_millis(at().as_millis() + 1).unwrap(),
            "00000000",
        )
        .unwrap();
        assert!(early < late);
    }

    #[test]
    fn a_wrong_kind_suffix_or_time_is_refused_naming_the_grammar() {
        for bad in [
            "g-20251019T123456789Z-1a2b3c4d",
            "t-20251019T123456789Z-1A2B3C4D",
            "t-20251019T123456789Z-1a2b3c4",
            "t-20251019T123456789Z-1a2b3c4dd",
            "t-2025-10-19T12:34:56.789Z-1a2b3c4d",
            "t-20251019T123456789Z",
            "t--1a2b3c4d",
            "",
        ] {
            let error = bad.parse::<TaskId>().unwrap_err();
            assert!(
                error.to_string().contains("t-<compact UTC>-<8 hex>"),
                "{bad}"
            );
        }
        assert!(TaskId::mint(at(), "XYZ").is_none());
    }

    #[test]
    fn a_fresh_id_parses_and_each_kind_has_its_prefix() {
        let fresh = MsgId::new_now();
        assert_eq!(fresh.to_string().parse::<MsgId>(), Ok(fresh.clone()));
        assert!(fresh.as_str().starts_with("msg-"));
        assert_eq!(random_suffix().len(), 8);
        let prefixes = [
            MsgId::PREFIX,
            TaskId::PREFIX,
            GateId::PREFIX,
            MemoId::PREFIX,
            MeetingId::PREFIX,
            BriefId::PREFIX,
            ActivationId::PREFIX,
        ];
        assert_eq!(prefixes, ["msg", "t", "g", "m", "mt", "brief", "act"]);
        // `m-` and `mt-` cannot be confused: the compact time follows the dash.
        assert!("mt-20251019T123456789Z-1a2b3c4d".parse::<MemoId>().is_err());
    }
}

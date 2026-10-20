//! One reference grammar (SDD-RUST §4.0 rule 4).
//!
//! An artifact reference is a path relative to the Agora's root
//! (`odeon/decks/…`). A log entry is `log:<seq>` and a message is `msg:<id>`.
//! Contract: an [`AgoraPath`] can never climb out of the Agora or name an
//! absolute path, on any platform, so a reference read from a file is safe to
//! join onto the Agora's root.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::id::MsgId;

/// A path inside the Agora, relative to its root, with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgoraPath(String);

/// A text that is not a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadRef(pub String);

impl fmt::Display for BadRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a reference: write a path relative to the Agora (odeon/decks/x.html), log:<seq>, or msg:<id>",
            self.0
        )
    }
}

impl AgoraPath {
    /// The path as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for AgoraPath {
    type Err = BadRef;

    fn from_str(text: &str) -> Result<AgoraPath, BadRef> {
        let segments_ok = !text.is_empty()
            && text.len() <= 1024
            && text.split('/').all(|segment| {
                !segment.is_empty()
                    && segment != "."
                    && segment != ".."
                    && !segment.contains(['\\', ':', '\0'])
            });
        if segments_ok {
            Ok(AgoraPath(text.to_string()))
        } else {
            Err(BadRef(text.to_string()))
        }
    }
}

impl fmt::Display for AgoraPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a record can point at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ref {
    /// A line of the book of record, by its sequence number.
    Log(u64),
    /// A Hermes message.
    Msg(MsgId),
    /// A file in the Agora.
    Path(AgoraPath),
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ref::Log(seq) => write!(f, "log:{seq}"),
            Ref::Msg(id) => write!(f, "msg:{id}"),
            Ref::Path(path) => f.write_str(path.as_str()),
        }
    }
}

impl FromStr for Ref {
    type Err = BadRef;

    fn from_str(text: &str) -> Result<Ref, BadRef> {
        if let Some(seq) = text.strip_prefix("log:") {
            let digits =
                !seq.is_empty() && seq.bytes().all(|b| b.is_ascii_digit()) && !seq.starts_with('0');
            return match (digits, seq.parse::<u64>()) {
                (true, Ok(n)) => Ok(Ref::Log(n)),
                _ => Err(BadRef(text.to_string())),
            };
        }
        if let Some(id) = text.strip_prefix("msg:") {
            return id
                .parse()
                .map(Ref::Msg)
                .map_err(|_| BadRef(text.to_string()));
        }
        text.parse().map(Ref::Path)
    }
}

macro_rules! string_serde {
    ($name:ident) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$name, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse()
                    .map_err(|error: BadRef| serde::de::Error::custom(error))
            }
        }
    };
}

string_serde!(AgoraPath);
string_serde!(Ref);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_a_log_line_a_message_or_an_agora_path() {
        assert_eq!("log:42".parse::<Ref>(), Ok(Ref::Log(42)));
        let msg: Ref = "msg:msg-20251019T123456789Z-1a2b3c4d".parse().unwrap();
        assert!(matches!(msg, Ref::Msg(_)));
        assert_eq!(msg.to_string(), "msg:msg-20251019T123456789Z-1a2b3c4d");
        let path: Ref = "odeon/decks/t-x.html".parse().unwrap();
        assert_eq!(path.to_string(), "odeon/decks/t-x.html");
    }

    #[test]
    fn a_path_cannot_leave_the_agora_or_be_absolute_on_any_platform() {
        for bad in [
            "",
            "/etc/passwd",
            "../secrets.enc",
            "odeon/../../x",
            "odeon//x",
            "./x",
            "C:\\x",
            "C:/x",
            "odeon\\x",
            "x/",
        ] {
            assert!(bad.parse::<AgoraPath>().is_err(), "{bad}");
        }
        assert!("knowledge/style.md".parse::<AgoraPath>().is_ok());
    }

    #[test]
    fn a_log_or_message_ref_must_be_well_formed() {
        for bad in [
            "log:",
            "log:-1",
            "log:01",
            "log:x",
            "msg:",
            "msg:t-20251019T123456789Z-1a2b3c4d",
        ] {
            let error = bad.parse::<Ref>().unwrap_err();
            assert!(error.to_string().contains("log:<seq>"), "{bad}");
        }
    }

    #[test]
    fn every_reference_prints_as_it_parses() {
        for text in [
            "log:7",
            "msg:msg-20251019T123456789Z-1a2b3c4d",
            "odeon/decks/x.html",
        ] {
            assert_eq!(text.parse::<Ref>().unwrap().to_string(), text);
        }
        let path: AgoraPath = "library/notes/a.md".parse().unwrap();
        assert_eq!(path.to_string(), "library/notes/a.md");
    }
}

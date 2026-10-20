//! The Hermes message: one JSON file per message in a mailbox (SDD-RUST §4.5;
//! SRS FR-3.1 as amended 2026-10-20; ADR-0003).
//!
//! Its field names are FR-3.1's, snake_case, because agents write messages and
//! the contract names them. `schemaVersion` is the one exception, spelled as
//! FR-3.1 spells it. Whether a message obligates a reply is a function of its
//! act ([`Message::requires_reply`], FR-3.3), not a field, since a stored copy
//! of it could disagree with the act.

use serde::{Deserialize, Serialize};

use crate::document::{Versioned, expect_version};
use crate::error::SchemaError;
use crate::id::MsgId;
use crate::names::{Address, AgentId, check_len};
use crate::time::Timestamp;

/// The speech acts (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Act {
    /// Asks the recipient to do something.
    Request,
    /// Tells the recipient something.
    Inform,
    /// Proposes something for the recipient to accept.
    Propose,
    /// Asks a question.
    Query,
    /// Accepts a request or proposal.
    Agree,
    /// Declines one, with the reason.
    Refuse,
    /// Reports a request finished.
    Done,
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// [`Message::VERSION`].
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    /// Its id, which is also its file name (`<id>.json`).
    pub id: MsgId,
    /// The conversation it belongs to, 1 to 64 characters.
    pub conversation: String,
    /// The message it answers, if any; never itself.
    pub in_reply_to: Option<MsgId>,
    /// Who sent it.
    pub from: AgentId,
    /// Where it goes.
    pub to: Address,
    /// What it does.
    pub act: Act,
    /// Its subject, 1 to 200 characters.
    pub subject: String,
    /// Its body, at most 200000 characters.
    pub body: String,
    /// Replies so far in its chain, 0 to 100; past the hop cap it escalates.
    pub hops: u8,
    /// Whether it needs the Architect.
    pub needs_human: bool,
    /// When it was written.
    pub created_at: Timestamp,
}

impl Message {
    /// Whether the act obligates a reply (FR-3.3): request, query and propose.
    pub fn requires_reply(&self) -> bool {
        matches!(self.act, Act::Request | Act::Query | Act::Propose)
    }

    /// The file name the message is stored under.
    pub fn file_name(&self) -> String {
        format!("{}.json", self.id)
    }
}

impl Versioned for Message {
    const FILE: &'static str = "message";
    const VERSION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
        check_len(Self::FILE, "conversation", &self.conversation, 1, 64)?;
        check_len(Self::FILE, "subject", &self.subject, 1, 200)?;
        check_len(Self::FILE, "body", &self.body, 0, 200_000)?;
        if self.hops > 100 {
            return Err(SchemaError::invalid(Self::FILE, "hops", "must be 0 to 100"));
        }
        if self.in_reply_to.as_ref() == Some(&self.id) {
            return Err(SchemaError::invalid(
                Self::FILE,
                "in_reply_to",
                "a message cannot answer itself",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{parse, render};

    const MESSAGE: &str = r#"{"schemaVersion":1,"id":"msg-20261019T123456789Z-1a2b3c4d","conversation":"c-1",
        "in_reply_to":null,"from":"agent.mason","to":"human","act":"request","subject":"review",
        "body":"please","hops":0,"needs_human":true,"created_at":"2026-10-19T12:34:56.789Z"}"#;

    #[test]
    fn a_message_uses_fr_3_1_s_names_and_round_trips() {
        let message = parse::<Message>(MESSAGE).unwrap();
        assert_eq!(message.to, Address::Human);
        assert_eq!(message.file_name(), "msg-20261019T123456789Z-1a2b3c4d.json");
        let text = render(&message).unwrap();
        for name in [
            "\"schemaVersion\"",
            "\"in_reply_to\"",
            "\"needs_human\"",
            "\"created_at\"",
        ] {
            assert!(text.contains(name), "{name}");
        }
        assert_eq!(parse::<Message>(&text).unwrap(), message);
    }

    #[test]
    fn whether_a_reply_is_owed_follows_the_act() {
        let mut message = parse::<Message>(MESSAGE).unwrap();
        for (act, owed) in [
            (Act::Request, true),
            (Act::Query, true),
            (Act::Propose, true),
            (Act::Inform, false),
            (Act::Agree, false),
            (Act::Refuse, false),
            (Act::Done, false),
        ] {
            message.act = act;
            assert_eq!(message.requires_reply(), owed, "{act:?}");
        }
    }

    #[test]
    fn requires_reply_and_a_missing_version_are_refused() {
        let with_field = MESSAGE.replace("\"hops\":0", "\"hops\":0,\"requires_reply\":true");
        assert!(
            parse::<Message>(&with_field)
                .unwrap_err()
                .to_string()
                .contains("requires_reply")
        );
        let unversioned = MESSAGE.replace("\"schemaVersion\":1,", "");
        assert!(matches!(
            parse::<Message>(&unversioned),
            Err(SchemaError::NoVersion { .. })
        ));
    }

    #[test]
    fn each_rule_refuses_naming_the_field() {
        let cases = [
            (
                MESSAGE.replace("\"subject\":\"review\"", "\"subject\":\"\""),
                "subject",
            ),
            (
                MESSAGE.replace("\"conversation\":\"c-1\"", "\"conversation\":\"\""),
                "conversation",
            ),
            (MESSAGE.replace("\"hops\":0", "\"hops\":101"), "hops"),
            (
                MESSAGE.replace(
                    "\"in_reply_to\":null",
                    "\"in_reply_to\":\"msg-20261019T123456789Z-1a2b3c4d\"",
                ),
                "answer itself",
            ),
            (
                MESSAGE.replace("\"from\":\"agent.mason\"", "\"from\":\"mason\""),
                "agent id",
            ),
            (
                MESSAGE.replace("\"to\":\"human\"", "\"to\":\"everyone\""),
                "address",
            ),
            (
                MESSAGE.replace("\"act\":\"request\"", "\"act\":\"demand\""),
                "unknown variant",
            ),
        ];
        for (text, needle) in cases {
            let error = parse::<Message>(&text).unwrap_err().to_string();
            assert!(error.contains(needle), "{needle}: {error}");
        }
    }
}

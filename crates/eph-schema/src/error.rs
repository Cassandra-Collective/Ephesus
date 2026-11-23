//! Why a document was refused.
//!
//! Every variant names the file and says what is allowed, because a refusal
//! must teach the rule rather than only state it (memory: *a refusal must teach
//! the rule*). The text is what the degradation channel shows the Architect.

use thiserror::Error;

/// A document that could not be accepted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SchemaError {
    /// The text is not JSON at all.
    #[error("{file}: not JSON ({detail}); the file is damaged")]
    NotJson {
        /// The document's file name.
        file: &'static str,
        /// serde_json's description, with its line and column.
        detail: String,
    },
    /// The JSON has no `schemaVersion`, or it is not a whole number.
    #[error(
        "{file}: no `schemaVersion`; add \"schemaVersion\": {expected}, the version this build reads (every Ephesus file carries its version as a whole number, invariant 9)"
    )]
    NoVersion {
        /// The document's file name.
        file: &'static str,
        /// The version this build reads, so the refusal can say what to write.
        expected: u32,
    },
    /// A version this build does not read.
    #[error(
        "{file}: schemaVersion {found} is not one this build reads (it reads {expected}); \
         {why}"
    )]
    UnknownVersion {
        /// The document's file name.
        file: &'static str,
        /// The version found.
        found: u64,
        /// The version this build writes and reads.
        expected: u32,
        /// What the Architect should do about it.
        why: &'static str,
    },
    /// The JSON's shape is wrong: a missing field, an unknown one, a wrong
    /// type, or one key written twice in an object.
    #[error("{file}: {detail}")]
    Shape {
        /// The document's file name.
        file: &'static str,
        /// serde's description: which field, what was expected.
        detail: String,
    },
    /// The shape is right and a value breaks a rule.
    #[error("{file}: {field}: {rule}")]
    Invalid {
        /// The document's file name.
        file: &'static str,
        /// The field, as a path (`agents.agent.x.cwd`).
        field: String,
        /// What is allowed.
        rule: String,
    },
}

impl SchemaError {
    /// An [`SchemaError::Invalid`] for `file`, `field` and `rule`.
    pub fn invalid(file: &'static str, field: impl Into<String>, rule: impl Into<String>) -> Self {
        SchemaError::Invalid {
            file,
            field: field.into(),
            rule: rule.into(),
        }
    }
}

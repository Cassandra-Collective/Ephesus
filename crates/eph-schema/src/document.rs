//! One version field, checked first (SDD-RUST §4.0 rule 1).
//!
//! Contract: [`parse`] reads a document in four steps, and each refuses with
//! its own [`SchemaError`]:
//!
//! 1. the text, strictly: not JSON, or an object naming a key twice, is refused
//!    ([`crate::json`]);
//! 2. the version, read before the shape, so a file from another build is
//!    reported as newer or older rather than as a pile of unknown fields;
//! 3. the shape, strict (unknown fields are refused);
//! 4. [`Versioned::validate`], every rule serde cannot express.
//!
//! Every persisted type implements [`Versioned`], the name ADR-0041's
//! invariant-9 row and RUST-PLAN §3.1 give it.
//!
//! [`render`] validates before it writes anything (rule 6), so no caller can
//! produce a file the next read would refuse. Neither function does I/O.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::SchemaError;

/// A persisted document with one version field (ADR-0041, invariant 9).
pub trait Versioned: Serialize + DeserializeOwned {
    /// The file's name, for messages (`registry.json`).
    const FILE: &'static str;
    /// The version this build writes and reads.
    const VERSION: u32;

    /// Every rule the type's shape does not already hold.
    fn validate(&self) -> Result<(), SchemaError>;
}

/// Parses and validates `text` as a `T`. See the module contract.
pub fn parse<T: Versioned>(text: &str) -> Result<T, SchemaError> {
    let value = crate::json::strict_value(text).map_err(|error| {
        if error.classify() == serde_json::error::Category::Data {
            SchemaError::Shape {
                file: T::FILE,
                detail: error.to_string(),
            }
        } else {
            SchemaError::NotJson {
                file: T::FILE,
                detail: error.to_string(),
            }
        }
    })?;
    check_version(T::FILE, T::VERSION, &value)?;
    let document: T = serde_json::from_value(value).map_err(|error| SchemaError::Shape {
        file: T::FILE,
        detail: error.to_string(),
    })?;
    document.validate()?;
    Ok(document)
}

/// The version check alone, for readers that need it before the shape: a log
/// line, whose reader keeps a line of a newer version as opaque.
pub fn check_version(
    file: &'static str,
    expected: u32,
    value: &serde_json::Value,
) -> Result<(), SchemaError> {
    let found = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or(SchemaError::NoVersion { file, expected })?;
    // One three-way comparison: a separate equality test followed by `>` left
    // a `>=` no test could tell apart (mutation round 1, 2026-10-20).
    let why = match found.cmp(&u64::from(expected)) {
        std::cmp::Ordering::Equal => return Ok(()),
        std::cmp::Ordering::Greater => {
            "a newer Ephesus wrote it; run that version, or restore an older copy of the file (a file in the Agora has one in its git history)"
        }
        std::cmp::Ordering::Less => {
            "an older Ephesus wrote it, such as the TypeScript build, and this build has no migration for it; start this build on another home, or rewrite the file in this version's shape"
        }
    };
    Err(SchemaError::UnknownVersion {
        file,
        found,
        expected,
        why,
    })
}

/// Validates `document` and renders it as pretty JSON with a final newline.
pub fn render<T: Versioned>(document: &T) -> Result<String, SchemaError> {
    document.validate()?;
    let mut text = serde_json::to_string_pretty(document).map_err(|error| SchemaError::Shape {
        file: T::FILE,
        detail: error.to_string(),
    })?;
    text.push('\n');
    Ok(text)
}

/// A document's own `schemaVersion` field must hold the version this build
/// reads; [`parse`] has already checked the text, and this checks a value
/// built in code.
pub fn expect_version(file: &'static str, expected: u32, found: u32) -> Result<(), SchemaError> {
    if found == expected {
        Ok(())
    } else {
        Err(SchemaError::invalid(
            file,
            "schemaVersion",
            format!("must be {expected}, the version this build writes"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    /// A document of version 2 whose one rule is `n` at most 9.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct Probe {
        schema_version: u32,
        n: u8,
    }

    impl Versioned for Probe {
        const FILE: &'static str = "probe.json";
        const VERSION: u32 = 2;

        fn validate(&self) -> Result<(), SchemaError> {
            expect_version(Self::FILE, Self::VERSION, self.schema_version)?;
            if self.n > 9 {
                return Err(SchemaError::invalid(Self::FILE, "n", "must be 0 to 9"));
            }
            Ok(())
        }
    }

    #[test]
    fn each_step_refuses_in_order_with_its_own_error() {
        assert!(matches!(
            parse::<Probe>("{"),
            Err(SchemaError::NotJson {
                file: "probe.json",
                ..
            })
        ));
        assert!(matches!(
            parse::<Probe>("{\"n\":1}"),
            Err(SchemaError::NoVersion {
                file: "probe.json",
                expected: 2
            })
        ));
        // The version is read before the shape: a newer file full of fields
        // this build does not know is reported as newer, not as unknown fields.
        let newer = parse::<Probe>("{\"schemaVersion\":3,\"whatever\":true}").unwrap_err();
        assert!(
            matches!(
                newer,
                SchemaError::UnknownVersion {
                    found: 3,
                    expected: 2,
                    ..
                }
            ),
            "{newer:?}"
        );
        assert!(
            newer.to_string().contains("a newer Ephesus wrote it"),
            "{newer}"
        );
        let older = parse::<Probe>("{\"schemaVersion\":1}").unwrap_err();
        assert!(
            matches!(
                older,
                SchemaError::UnknownVersion {
                    found: 1,
                    expected: 2,
                    ..
                }
            ),
            "{older:?}"
        );
        assert!(
            older.to_string().contains(
                "an older Ephesus wrote it, such as the TypeScript build, and this build has no migration"
            ),
            "{older}"
        );
        assert!(matches!(
            parse::<Probe>("{\"schemaVersion\":2,\"n\":1,\"x\":0}"),
            Err(SchemaError::Shape {
                file: "probe.json",
                ..
            })
        ));
        let invalid = parse::<Probe>("{\"schemaVersion\":2,\"n\":10}").unwrap_err();
        assert_eq!(invalid.to_string(), "probe.json: n: must be 0 to 9");
        assert_eq!(
            parse::<Probe>("{\"schemaVersion\":2,\"n\":1}").unwrap(),
            Probe {
                schema_version: 2,
                n: 1
            }
        );
    }

    #[test]
    fn render_validates_first_and_ends_with_one_newline() {
        let text = render(&Probe {
            schema_version: 2,
            n: 1,
        })
        .unwrap();
        assert_eq!(text, "{\n  \"schemaVersion\": 2,\n  \"n\": 1\n}\n");
        assert!(
            render(&Probe {
                schema_version: 2,
                n: 10
            })
            .is_err()
        );
        let wrong = render(&Probe {
            schema_version: 1,
            n: 1,
        })
        .unwrap_err()
        .to_string();
        assert_eq!(
            wrong,
            "probe.json: schemaVersion: must be 2, the version this build writes"
        );
    }

    #[test]
    fn a_field_written_twice_is_a_shape_error_naming_it_and_where() {
        // Review finding F1: serde_json keeps the last of two equal keys, and
        // reading through a `Value` also bypassed serde-derive's own refusal of
        // a repeated field.
        let error = parse::<Probe>("{\"schemaVersion\":2,\"n\":1,\n\"n\":9}").unwrap_err();
        assert!(
            matches!(
                error,
                SchemaError::Shape {
                    file: "probe.json",
                    ..
                }
            ),
            "{error:?}"
        );
        let text = error.to_string();
        assert!(text.contains("`n` appears twice in one object"), "{text}");
        assert!(text.contains("line 2"), "{text}");
        let version_twice = "{\"schemaVersion\":2,\"schemaVersion\":3,\"n\":1}";
        assert!(matches!(
            parse::<Probe>(version_twice),
            Err(SchemaError::Shape { .. })
        ));
    }

    #[test]
    fn a_version_that_is_not_a_whole_number_is_no_version() {
        for value in [
            serde_json::json!({"schemaVersion": "2"}),
            serde_json::json!({"schemaVersion": 2.5}),
            serde_json::json!([2]),
        ] {
            assert!(
                matches!(
                    check_version("f", 2, &value),
                    Err(SchemaError::NoVersion {
                        file: "f",
                        expected: 2
                    })
                ),
                "{value}"
            );
        }
        assert!(check_version("f", 2, &serde_json::json!({"schemaVersion": 2})).is_ok());
    }
}

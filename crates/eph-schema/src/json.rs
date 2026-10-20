//! JSON read strictly: an object that names a key twice is refused.
//!
//! Contract: [`strict_value`] parses text as `serde_json` does, except that a
//! repeated key in any object, at any depth, is an error naming the key and
//! where it is. `serde_json` keeps the last of two equal keys, so before this a
//! second gate rule for one kind silently overruled the first, and a repeated
//! agent id dropped a hire (review finding F1, 2026-10-20). Every read of a
//! persisted document and of a log line goes through here. Pure: no I/O.

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// Parses `text` into a JSON value, refusing a repeated key. A refusal is a
/// `serde_json::Error` of category `Data` for a repeated key and of `Syntax` or
/// `Eof` for text that is not JSON, with the line and column either way.
pub fn strict_value(text: &str) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = Strict.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

struct Strict;

impl<'de> DeserializeSeed<'de> for Strict {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::from(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("a number JSON cannot hold"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Strict)? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!(
                    "`{key}` appears twice in one object; keep the one you mean and delete the other"
                )));
            }
            let value = map.next_value_seed(Strict)?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_key_is_refused_at_any_depth_naming_it_and_where() {
        for (text, key) in [
            (r#"{"a":1,"a":2}"#, "a"),
            (r#"{"x":{"rules":{"k":1,"k":1}}}"#, "k"),
            (r#"[{"y":true},{"z":null,"z":null}]"#, "z"),
        ] {
            let error = strict_value(text).unwrap_err();
            assert_eq!(
                error.classify(),
                serde_json::error::Category::Data,
                "{text}"
            );
            let message = error.to_string();
            assert!(
                message.contains(&format!("`{key}` appears twice in one object")),
                "{message}"
            );
            assert!(message.contains("line 1 column"), "{message}");
        }
    }

    #[test]
    fn everything_else_reads_as_serde_json_reads_it() {
        let text = r#"{"s":"t","n":-3,"u":18446744073709551615,"f":1.5,"b":false,"z":null,"a":[1,{"k":[]}],"e":{}}"#;
        let expected: Value = serde_json::from_str(text).unwrap();
        assert_eq!(strict_value(text).unwrap(), expected);
        assert_eq!(strict_value(r#"{"a":1,"b":1}"#).unwrap()["b"], 1);
        for bad in ["{", "", "{} x", "[1,]"] {
            let error = strict_value(bad).unwrap_err();
            assert_ne!(
                error.classify(),
                serde_json::error::Category::Data,
                "{bad:?}"
            );
        }
    }
}

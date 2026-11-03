use super::{Failure, MAX_SAFE_INTEGER};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

pub fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn canonical_json(value: &Value, limit: usize) -> Result<Vec<u8>, Failure> {
    check(value)?;
    let bytes = serde_json_canonicalizer::to_vec(value).map_err(|_| Failure::GateInvalidInput)?;
    if bytes.len() > limit {
        return Err(Failure::GateLimit);
    }
    Ok(bytes)
}

pub fn parse_json(bytes: &[u8], limit: usize) -> Result<Value, Failure> {
    if bytes.len() > limit {
        return Err(Failure::GateLimit);
    }
    let StrictJson(value) = serde_json::from_slice(bytes).map_err(|_| Failure::GateInvalidInput)?;
    check(&value)?;
    Ok(value)
}

fn check(value: &Value) -> Result<(), Failure> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), Failure> {
        *nodes += 1;
        if depth > 32 || *nodes > 4096 {
            return Err(Failure::GateLimit);
        }
        match value {
            Value::Number(number)
                if number
                    .as_i64()
                    .is_some_and(|n| n.unsigned_abs() <= MAX_SAFE_INTEGER)
                    || number.as_u64().is_some_and(|n| n <= MAX_SAFE_INTEGER) => {}
            Value::Number(_) => return Err(Failure::GateInvalidInput),
            Value::Array(values) => {
                for value in values {
                    visit(value, depth + 1, nodes)?;
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    visit(value, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, 0, &mut 0)
}
struct StrictJson(Value);
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}
struct JsonVisitor;
impl<'de> Visitor<'de> for JsonVisitor {
    type Value = StrictJson;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded integer JSON with unique keys")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Bool(v)))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        if v.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(E::custom("unsafe integer"));
        }
        Ok(StrictJson(v.into()))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        if v > MAX_SAFE_INTEGER {
            return Err(E::custom("unsafe integer"));
        }
        Ok(StrictJson(v.into()))
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Err(E::custom("integer lexeme required"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(StrictJson(v.into()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        Ok(StrictJson(v.into()))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictJson(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(StrictJson(v)) = seq.next_element()? {
            if values.len() == 4096 {
                return Err(de::Error::custom("JSON node limit"));
            }
            values.push(v);
        }
        Ok(StrictJson(values.into()))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.len() == 4096 || values.contains_key(&key) {
                return Err(de::Error::custom("duplicate key or JSON node limit"));
            }
            let StrictJson(v) = map.next_value()?;
            values.insert(key, v);
        }
        Ok(StrictJson(values.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn duplicate_keys_decimals_unsafe_integers_and_deep_values_are_rejected() {
        for input in [
            r#"{"a":1,"a":2}"#,
            r#"{"a":1.0}"#,
            r#"{"a":9007199254740992}"#,
        ] {
            assert!(parse_json(input.as_bytes(), 1024).is_err());
        }
        assert!(parse_json(br#"{"ok":1}"#, 4).is_err());
        let mut deep = json!({});
        for _ in 0..33 {
            deep = json!({"nested":deep});
        }
        assert!(canonical_json(&deep, 4096).is_err());
    }
    #[test]
    fn canonical_configuration_identity_is_independent_of_key_order() {
        assert_eq!(
            canonical_json(&json!({"b":2,"a":1}), 1024).unwrap(),
            br#"{"a":1,"b":2}"#
        );
    }
}

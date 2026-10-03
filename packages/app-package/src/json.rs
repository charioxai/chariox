use std::fmt;

use serde::{
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::Value;

use crate::{ErrorCode, Limits, PackageError, Result};

pub(crate) fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json_canonicalizer::to_vec(value)
        .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "JSON canonicalization failed"))
}

pub(crate) fn read_json<T: DeserializeOwned>(
    bytes: &[u8],
    max_bytes: usize,
    limits: &Limits,
    code: ErrorCode,
    require_canonical: bool,
) -> Result<T> {
    if bytes.len() > max_bytes {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "JSON document exceeds byte limit",
        ));
    }
    let StrictValue(value) = serde_json::from_slice(bytes)
        .map_err(|_| PackageError::new(code, "invalid JSON or duplicate object key"))?;
    check_value(&value, limits)?;
    if require_canonical && canonical(&value)? != bytes {
        return Err(PackageError::new(
            code,
            "control JSON must use RFC 8785 canonical encoding",
        ));
    }
    serde_json::from_value(value)
        .map_err(|_| PackageError::new(code, "JSON does not match the declared contract"))
}

pub(crate) fn check_value(value: &Value, limits: &Limits) -> Result<()> {
    fn visit(value: &Value, depth: usize, count: &mut usize, limits: &Limits) -> Result<()> {
        *count += 1;
        if depth > limits.max_json_depth || *count > limits.max_json_nodes {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "JSON depth or node count exceeds limit",
            ));
        }
        match value {
            Value::Array(items) => {
                for item in items {
                    visit(item, depth + 1, count, limits)?;
                }
            }
            Value::Object(object) => {
                for item in object.values() {
                    visit(item, depth + 1, count, limits)?;
                }
            }
            Value::Number(number) => {
                const MAX_SAFE: u64 = 9_007_199_254_740_991;
                if number.as_u64().is_some_and(|n| n > MAX_SAFE)
                    || number.as_i64().is_some_and(|n| n < -(MAX_SAFE as i64))
                    || number
                        .as_f64()
                        .is_some_and(|n| n.fract() == 0.0 && n.abs() > MAX_SAFE as f64)
                {
                    return Err(PackageError::new(
                        ErrorCode::InvalidSchema,
                        "integers must fit the interoperable JSON number range",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, 0, &mut 0, limits)
}

/// serde_json::Value accepts duplicate keys. Package declarations must not have
/// different meanings in downstream schema, signature, or language parsers.
struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_string<E: de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut array = Vec::new();
                while let Some(StrictValue(value)) = sequence.next_element()? {
                    array.push(value);
                }
                Ok(StrictValue(Value::Array(array)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut access: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut object = serde_json::Map::new();
                while let Some((key, StrictValue(value))) =
                    access.next_entry::<String, StrictValue>()?
                {
                    if object.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate object key"));
                    }
                }
                Ok(StrictValue(Value::Object(object)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

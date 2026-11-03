//! Publisher-signed event data. Uses the same offline Draft 7 compiler as Apps.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAX_DATA_BYTES: usize = 64 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventPayloadExample {
    pub name: String,
    pub data: Value,
}

pub fn validate_event_data(data: Option<&Value>) -> Result<(), String> {
    let Some(data) = data else { return Ok(()) };
    if !data.is_object() {
        return Err("event data must be an object".into());
    }
    bounded_json(data, MAX_DATA_BYTES)
}

pub fn bounded_json(value: &Value, max_bytes: usize) -> Result<(), String> {
    fn visit(v: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
        *nodes += 1;
        if depth > 32 || *nodes > 4096 {
            return Err("event JSON exceeds depth/node limit".into());
        }
        match v {
            Value::Number(n)
                if n.as_i64()
                    .is_some_and(|n| n.unsigned_abs() <= 9_007_199_254_740_991)
                    || n.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991) => {}
            Value::Number(_) => {
                return Err("event JSON requires safe integer numeric literals".into())
            }
            Value::Object(o) => {
                for v in o.values() {
                    visit(v, depth + 1, nodes)?;
                }
            }
            Value::Array(a) => {
                for v in a {
                    visit(v, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(value, 0, &mut 0)?;
    if serde_json::to_vec(value)
        .map_err(|_| "invalid event JSON")?
        .len()
        > max_bytes
    {
        return Err("event JSON exceeds byte limit".into());
    }
    Ok(())
}

pub fn payload_schema_digest(schema: &Value) -> Result<String, String> {
    let bytes = serde_json_canonicalizer::to_vec(schema).map_err(|_| "invalid schema JSON")?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub fn validate_payload_definition(
    schema: Option<&Value>,
    examples: &[EventPayloadExample],
) -> Result<(), String> {
    let Some(schema) = schema else {
        return if examples.is_empty() {
            Ok(())
        } else {
            Err("event examples require payload_schema".into())
        };
    };
    bounded_json(schema, MAX_DATA_BYTES)?;
    let limits = chariox_app_package::Limits {
        max_schema_bytes: MAX_DATA_BYTES,
        max_json_nodes: 4096,
        ..Default::default()
    };
    let compiled =
        chariox_app_package::compile_schema(schema, true, &limits).map_err(|e| e.to_string())?;
    if examples.len() > 8 {
        return Err("event examples exceed 8 items".into());
    }
    let mut names = std::collections::BTreeSet::new();
    let mut total = 0;
    for example in examples {
        if example.name.trim().is_empty()
            || example.name.len() > 128
            || !names.insert(&example.name)
        {
            return Err("event example names must be nonempty, bounded and unique".into());
        }
        validate_event_data(Some(&example.data))?;
        bounded_json(&example.data, 8192)?;
        total += serde_json::to_vec(example)
            .map_err(|_| "invalid example")?
            .len();
        if total > 32768 || !compiled.is_valid(&example.data) {
            return Err("event example exceeds limits or does not match payload_schema".into());
        }
    }
    Ok(())
}

pub fn validate_payload(schema: Option<&Value>, data: Option<&Value>) -> Result<(), String> {
    validate_event_data(data)?;
    match (schema, data) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err("text-only event cannot carry typed data".into()),
        (Some(_), None) => Err("typed event requires data".into()),
        (Some(schema), Some(data)) => {
            validate_payload_definition(Some(schema), &[])?;
            let limits = chariox_app_package::Limits {
                max_schema_bytes: MAX_DATA_BYTES,
                max_json_nodes: 4096,
                ..Default::default()
            };
            let compiled = chariox_app_package::compile_schema(schema, true, &limits)
                .map_err(|e| e.to_string())?;
            if compiled.is_valid(data) {
                Ok(())
            } else {
                Err("event data does not match payload_schema".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,"required":["severity"],"properties":{"severity":{"enum":["high","low"]}}})
    }
    #[test]
    fn signed_examples_and_delivered_data_use_the_same_closed_schema() {
        let examples = [EventPayloadExample {
            name: "High".into(),
            data: json!({"severity":"high"}),
        }];
        validate_payload_definition(Some(&schema()), &examples).unwrap();
        validate_payload(Some(&schema()), Some(&examples[0].data)).unwrap();
        assert!(validate_payload(Some(&schema()), Some(&json!({}))).is_err());
        assert!(
            validate_payload(Some(&schema()), Some(&json!({"severity":"high","extra":1}))).is_err()
        );
        assert!(validate_payload_definition(None, &examples).is_err());
        assert!(validate_payload(None, Some(&json!({}))).is_err());
        assert!(validate_payload(Some(&schema()), None).is_err());
    }
    #[test]
    fn reject_remote_recursive_and_unbounded_schemas_and_examples() {
        for schema in [
            json!({"$ref":"https://example.com/schema"}),
            json!({"type":"object","additionalProperties":false,"properties":{"self":{"$ref":"#"}}}),
            json!({"type":"object"}),
        ] {
            assert!(validate_payload_definition(Some(&schema), &[]).is_err());
        }
        assert!(validate_event_data(Some(&json!({"value":1.2}))).is_err());
        assert!(validate_event_data(Some(&json!({"value":9007199254740992u64}))).is_err());
        assert!(validate_payload_definition(
            Some(&schema()),
            &[EventPayloadExample {
                name: "Bad".into(),
                data: json!({"severity":"unknown"})
            }]
        )
        .is_err());
    }
}

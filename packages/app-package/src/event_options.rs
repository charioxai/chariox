//! Subscription options an event declares in its payload schema. A subscriber picks values
//! for them ("PR number 976"), and the kernel compares each occurrence's declared field
//! against those values before anyone is woken.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{manifest::valid_function_name, ErrorCode, PackageError, Result};

/// Root keyword of a payload schema that lists the event's subscription options.
pub const EVENT_OPTIONS_KEYWORD: &str = "x-chariox-options";
pub const MAX_EVENT_OPTIONS: usize = 4;
const MAX_OPTION_LABEL_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventOption {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: EventOptionType,
    /// Top-level payload property the option compares against.
    pub field: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventOptionType {
    Number,
    Text,
    Boolean,
}

impl EventOptionType {
    fn schema_type(self) -> &'static str {
        match self {
            Self::Number => "integer",
            Self::Text => "string",
            Self::Boolean => "boolean",
        }
    }

    /// Whether `value` is a value of this option type.
    pub fn admits(self, value: &Value) -> bool {
        match self {
            Self::Number => value.is_i64() || value.is_u64(),
            Self::Text => value.is_string(),
            Self::Boolean => value.is_boolean(),
        }
    }
}

/// The options a payload schema declares; none when it declares none. Each option names a
/// top-level property whose schema type matches the option type.
pub fn event_options(schema: &Value) -> Result<Vec<EventOption>> {
    let Some(declared) = schema.get(EVENT_OPTIONS_KEYWORD) else {
        return Ok(Vec::new());
    };
    let options: Vec<EventOption> = serde_json::from_value(declared.clone())
        .map_err(|_| invalid("x-chariox-options must list {name, label, type, field} options"))?;
    if options.len() > MAX_EVENT_OPTIONS {
        return Err(invalid("x-chariox-options lists more than 4 options"));
    }
    for (index, option) in options.iter().enumerate() {
        if !valid_function_name(&option.name)
            || options[..index].iter().any(|seen| seen.name == option.name)
        {
            return Err(invalid(
                "x-chariox-options names must be unique lowercase identifiers",
            ));
        }
        if option.label.trim().is_empty() || option.label.chars().count() > MAX_OPTION_LABEL_CHARS {
            return Err(invalid(
                "x-chariox-options labels must be 1 to 64 characters",
            ));
        }
        let field_type = schema
            .pointer(&format!("/properties/{}/type", option.field))
            .and_then(Value::as_str);
        if option.field.contains('/') || field_type != Some(option.kind.schema_type()) {
            return Err(invalid(
                "each x-chariox-options field must be a top-level payload property of the option's type",
            ));
        }
    }
    Ok(options)
}

/// One occurrence's values for the declared options, keyed by option name. A field the
/// payload lacks, or holds with another type, is left out.
pub fn option_values(options: &[EventOption], payload: &Value) -> Map<String, Value> {
    options
        .iter()
        .filter_map(|option| {
            let value = payload.get(&option.field)?;
            option
                .kind
                .admits(value)
                .then(|| (option.name.clone(), value.clone()))
        })
        .collect()
}

fn invalid(message: &str) -> PackageError {
    PackageError::new(ErrorCode::InvalidSchema, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(options: Value) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "pull_request": {"type": "integer"},
                "repository": {"type": "string"},
                "draft": {"type": "boolean"}
            },
            "x-chariox-options": options
        })
    }

    #[test]
    fn declared_options_are_read_and_pick_their_fields_from_a_payload() {
        let options = event_options(&schema(json!([
            {"name": "pr", "label": "PR number", "type": "number", "field": "pull_request"},
            {"name": "repo", "label": "Repository", "type": "text", "field": "repository"}
        ])))
        .unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].kind, EventOptionType::Number);
        assert_eq!(
            Value::Object(option_values(
                &options,
                &json!({"pull_request": 976, "repository": 7, "title": "x"})
            )),
            json!({"pr": 976})
        );
        assert!(event_options(&json!({"type": "object"}))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn options_that_cannot_be_matched_are_refused() {
        for options in [
            json!({"name": "pr"}),
            json!([{"name": "pr", "label": "PR", "type": "number", "field": "missing"}]),
            json!([{"name": "pr", "label": "PR", "type": "text", "field": "pull_request"}]),
            json!([
                {"name": "pr", "label": "PR", "type": "number", "field": "pull_request"},
                {"name": "pr", "label": "Again", "type": "text", "field": "repository"}
            ]),
            json!([{"name": "PR", "label": "PR", "type": "number", "field": "pull_request"}]),
            json!([{"name": "pr", "label": " ", "type": "number", "field": "pull_request"}]),
            json!([{"name": "pr", "label": "PR", "type": "date", "field": "pull_request"}]),
            json!((0..5)
                .map(|i| json!({"name": format!("o{i}"), "label": "x", "type": "boolean", "field": "draft"}))
                .collect::<Vec<_>>()),
        ] {
            assert!(event_options(&schema(options.clone())).is_err(), "{options}");
        }
    }
}

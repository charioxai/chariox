//! MP-08 / MP-10 / MP-11: provider control context is not a user turn.
use serde_json::Value;

pub(super) fn opencode_internal_summary(role: Option<&str>, info: &Value) -> bool {
    role == Some("assistant") && info.get("summary").and_then(Value::as_bool) == Some(true)
}

pub(super) fn opencode_internal_user_part(role: Option<&str>, part: &Value) -> bool {
    role == Some("user") && part.get("synthetic").and_then(Value::as_bool) == Some(true)
}

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::error::DaemonError;

const AUTH_MARKER: &str = "CHARIOX_AUTH_OK";

// setup-token credentials can make model requests, but cannot fetch account
// usage. Verify through the official CLI with one small turn and no tools.
pub(crate) fn verify_claude_setup_token(
    executable: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(), super::usage_probe::ClaudeCredentialCheckError> {
    super::usage_probe::verify_claude_account_credential_with_timeout(
        executable,
        environment,
        Duration::from_secs(30),
    )
}

pub(super) fn validate_auth_result(value: &serde_json::Value) -> Result<(), DaemonError> {
    let verified = value.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && value.get("subtype").and_then(serde_json::Value::as_str) == Some("success")
        && value.get("is_error").and_then(serde_json::Value::as_bool) == Some(false)
        && value.get("num_turns").and_then(serde_json::Value::as_u64) == Some(1)
        && value
            .get("result")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.trim() == AUTH_MARKER)
        && value
            .get("usage")
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count > 0);
    if verified {
        return Ok(());
    }
    // Provider result text can contain account details. Never include it in
    // errors, logs or runtime interactions.
    Err(DaemonError::LocalTransport {
        operation: "verify Claude setup token",
        message: "Claude did not complete the authentication verification turn".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn successful_turn() -> serde_json::Value {
        serde_json::json!({"type":"result", "subtype":"success", "is_error":false,
            "num_turns":1, "result":AUTH_MARKER, "usage":{"output_tokens":7}})
    }

    #[test]
    fn requires_a_successful_single_model_turn() {
        assert!(validate_auth_result(&successful_turn()).is_ok());
        for (field, value) in [
            ("type", serde_json::json!("system")),
            ("subtype", serde_json::json!("error_during_execution")),
            ("is_error", serde_json::json!(true)),
            ("num_turns", serde_json::json!(0)),
            ("num_turns", serde_json::json!(2)),
            (
                "result",
                serde_json::json!("provider account details must stay private"),
            ),
            ("usage", serde_json::json!({"output_tokens":0})),
        ] {
            let mut result = successful_turn();
            result[field] = value;
            let error = validate_auth_result(&result).expect_err(field).to_string();
            assert!(!error.contains("provider account details"));
        }
    }

    #[test]
    fn rejects_local_slash_command_success_as_authentication() {
        let result = serde_json::json!({"type":"result", "subtype":"success",
            "is_error":false, "num_turns":0, "result":AUTH_MARKER,
            "usage":{"output_tokens":0}});
        assert!(validate_auth_result(&result).is_err());
    }
}

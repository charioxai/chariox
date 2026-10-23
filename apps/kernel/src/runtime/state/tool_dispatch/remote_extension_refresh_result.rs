use crate::error::DaemonError;
use crate::transport::runtime_tools::RuntimeToolResult;

pub(super) fn preserve_original_result(
    original_response: Option<RuntimeToolResult>,
    refresh: Result<(), DaemonError>,
) -> Result<RuntimeToolResult, DaemonError> {
    let Some(mut result) = original_response else {
        return Err(refresh.expect_err("missing response requires a forwarding error"));
    };
    if let Err(error) = refresh {
        // A read-only refresh failure must not invite replay of an operation
        // whose result the home kernel already returned.
        let warning = serde_json::json!({
            "error": error.to_string(),
            "original_tool_replayed": false,
            "original_result_preserved": true,
        });
        if let Some(payload) = result
            .payload
            .as_object_mut()
            .filter(|payload| !payload.contains_key("manifest_refresh_warning"))
        {
            payload.insert("manifest_refresh_warning".to_string(), warning);
        } else {
            result.payload = serde_json::json!({
                "original_payload": result.payload,
                "manifest_refresh_warning": warning,
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn error() -> DaemonError {
        DaemonError::LocalTransport {
            operation: "forward leased capability runtime tool",
            message: "read-only refresh failed".into(),
        }
    }
    #[test]
    fn refresh_failure_preserves_original_success_and_denial_without_replay() {
        for ok in [true, false] {
            let result = preserve_original_result(
                Some(RuntimeToolResult {
                    ok,
                    payload: json!({"original":true}),
                }),
                Err(error()),
            )
            .unwrap();
            assert_eq!(result.ok, ok);
            assert_eq!(result.payload["original"], true);
            assert_eq!(
                result.payload["manifest_refresh_warning"]["original_tool_replayed"],
                false
            );
        }
    }
    #[test]
    fn refresh_collision_and_nonobject_payloads_are_preserved() {
        for payload in [
            json!({"manifest_refresh_warning":"original"}),
            json!(null),
            json!([42]),
            json!(false),
        ] {
            let result = preserve_original_result(
                Some(RuntimeToolResult {
                    ok: true,
                    payload: payload.clone(),
                }),
                Err(error()),
            )
            .unwrap();
            assert_eq!(result.payload["original_payload"], payload);
            assert_eq!(
                result.payload["manifest_refresh_warning"]["original_result_preserved"],
                true
            );
        }
    }
    #[test]
    fn first_request_failure_remains_an_error() {
        assert!(preserve_original_result(None, Err(error())).is_err());
    }
    #[test]
    fn successful_refresh_leaves_the_original_result_unchanged() {
        let original = RuntimeToolResult {
            ok: false,
            payload: json!(["original denial"]),
        };
        assert_eq!(
            preserve_original_result(Some(original.clone()), Ok(())).unwrap(),
            original
        );
    }
}

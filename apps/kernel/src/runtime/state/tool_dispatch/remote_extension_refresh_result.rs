use crate::error::DaemonError;
use crate::skill::CharioxSkillPackage;
use crate::transport::runtime_tools::RuntimeToolResult;

pub(super) fn preserve_original_result(
    original_response: Option<(RuntimeToolResult, Option<CharioxSkillPackage>)>,
    refresh: Result<(), DaemonError>,
) -> Result<(RuntimeToolResult, Option<CharioxSkillPackage>), DaemonError> {
    let Some((mut result, skill_package)) = original_response else {
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
    Ok((result, skill_package))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn transport_error(message: &str) -> DaemonError {
        DaemonError::LocalTransport {
            operation: "forward leased capability runtime tool",
            message: message.to_string(),
        }
    }

    fn package() -> CharioxSkillPackage {
        CharioxSkillPackage {
            metadata: crate::skill::CharioxSkillMetadata {
                name: "original-skill".to_string(),
                description: "Original response package".to_string(),
                short_description: None,
                path: "original-skill/SKILL.md".into(),
            },
            version_hash: "original-version".to_string(),
            files: Vec::new(),
        }
    }

    fn assert_warning(payload: &Value, error: &str) {
        assert_eq!(
            payload["manifest_refresh_warning"],
            json!({
                "error": error,
                "original_tool_replayed": false,
                "original_result_preserved": true,
            })
        );
    }

    #[test]
    fn warning_key_collision_preserves_original_payload() {
        let payload = json!({"manifest_refresh_warning": "original", "granted": true});
        let (result, _) = preserve_original_result(
            Some((
                RuntimeToolResult {
                    ok: true,
                    payload: payload.clone(),
                },
                None,
            )),
            Err(transport_error("refresh failed")),
        )
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.payload["original_payload"], payload);
        assert_eq!(
            result.payload["manifest_refresh_warning"]["original_result_preserved"],
            true
        );
    }

    #[test]
    fn first_request_error_remains_an_error() {
        let error = transport_error("first request disconnected");
        let expected = error.to_string();
        let actual = preserve_original_result(None, Err(error)).unwrap_err();
        assert_eq!(actual.to_string(), expected);
    }

    #[test]
    fn refresh_transport_failure_preserves_original_success_and_package() {
        let original_package = package();
        let error = transport_error("read-only refresh disconnected");
        let expected = error.to_string();
        let (result, retained_package) = preserve_original_result(
            Some((
                RuntimeToolResult {
                    ok: true,
                    payload: json!({"registered": "original"}),
                },
                Some(original_package.clone()),
            )),
            Err(error),
        )
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.payload["registered"], "original");
        assert_eq!(retained_package, Some(original_package));
        assert_warning(&result.payload, &expected);
    }

    #[test]
    fn unexpected_refresh_response_preserves_original_result() {
        let error = transport_error("unexpected forwarded capability response: Ack");
        let expected = error.to_string();
        let (result, package) = preserve_original_result(
            Some((
                RuntimeToolResult {
                    ok: true,
                    payload: json!({"granted": true}),
                },
                None,
            )),
            Err(error),
        )
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.payload["granted"], true);
        assert!(package.is_none());
        assert_warning(&result.payload, &expected);
    }

    #[test]
    fn failed_original_result_is_not_replaced_by_refresh_failure() {
        let error = transport_error("refresh failed");
        let expected = error.to_string();
        let (result, _) = preserve_original_result(
            Some((
                RuntimeToolResult {
                    ok: false,
                    payload: json!({"error": "original grant denied"}),
                },
                None,
            )),
            Err(error),
        )
        .unwrap();
        assert!(!result.ok);
        assert_eq!(result.payload["error"], "original grant denied");
        assert_warning(&result.payload, &expected);
    }

    #[test]
    fn non_object_payloads_are_retained_without_coercion() {
        for payload in [
            Value::Null,
            json!(false),
            json!(42),
            json!("original"),
            json!([1, {"nested": true}]),
        ] {
            let error = transport_error("refresh failed");
            let expected = error.to_string();
            let (result, _) = preserve_original_result(
                Some((
                    RuntimeToolResult {
                        ok: true,
                        payload: payload.clone(),
                    },
                    None,
                )),
                Err(error),
            )
            .unwrap();
            assert!(result.ok);
            assert_eq!(result.payload["original_payload"], payload);
            assert_warning(&result.payload, &expected);
        }
    }

    #[test]
    fn successful_refresh_leaves_original_result_and_package_unchanged() {
        let original = (
            RuntimeToolResult {
                ok: false,
                payload: json!(["original denial"]),
            },
            Some(package()),
        );
        assert_eq!(
            preserve_original_result(Some(original.clone()), Ok(())).unwrap(),
            original
        );
    }
}

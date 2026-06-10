use crate::error::DaemonError;

use super::{slice_tool_payload, SliceScreenCommandOutput};

pub(super) fn slice_browser_tool_result(
    slice_id: &str,
    agent_id: &str,
    output: SliceScreenCommandOutput,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let mut payload = slice_tool_payload(slice_id, agent_id, &output);
    if let Ok(browser) = slice_browser_json(&output) {
        payload["browser"] = browser;
    }
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: output.success,
        payload,
    }
}

pub(super) fn slice_browser_json(
    output: &SliceScreenCommandOutput,
) -> Result<serde_json::Value, DaemonError> {
    serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "slice_browser_json",
            message: format!("slice browser command did not return JSON: {error}"),
        }
    })
}

pub(super) fn browser_status_url(status: &serde_json::Value) -> Result<String, DaemonError> {
    status
        .get("url")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "runtime_tool_paste_secret_to_slice",
            message: "slice browser status did not include a current URL".to_string(),
        })
}

pub(super) fn ensure_browser_target_matches_expectations(
    status: &serde_json::Value,
    args: &crate::transport::runtime_tools::PasteSecretToSliceArgs,
) -> Result<(), DaemonError> {
    let current_url = browser_status_url(status)?;
    if let Some(expected_url) = args.expected_url.as_deref().map(str::trim) {
        if !expected_url.is_empty() && !current_url.starts_with(expected_url) {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_paste_secret_to_slice",
                message: format!(
                    "slice browser URL `{current_url}` does not match expected URL prefix `{expected_url}`"
                ),
            });
        }
    }
    if let Some(expected_host) = args.expected_host.as_deref().map(str::trim) {
        if !expected_host.is_empty() {
            let url =
                url::Url::parse(&current_url).map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_paste_secret_to_slice",
                    message: format!("slice browser URL is invalid: {error}"),
                })?;
            let host = url.host_str().unwrap_or("");
            let host_with_port = match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            };
            if expected_host != host && expected_host != host_with_port {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_paste_secret_to_slice",
                    message: format!(
                        "slice browser host `{host_with_port}` does not match expected host `{expected_host}`"
                    ),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_browser_fill_target(
    status: &serde_json::Value,
    selector: Option<&str>,
) -> Result<(), DaemonError> {
    if let Some(selector) = selector.map(str::trim).filter(|value| !value.is_empty()) {
        let found = status
            .get("fields")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|fields| {
                fields.iter().any(|field| {
                    field
                        .get("selector")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|field_selector| field_selector == selector)
                })
            });
        if found {
            return Ok(());
        }
        return Err(DaemonError::LocalTransport {
            operation: "runtime_tool_paste_secret_to_slice",
            message: format!("slice browser field `{selector}` was not found or is not fillable"),
        });
    }

    let focused = status.get("focusedElement");
    let focused_kind = focused
        .and_then(|value| value.get("kind"))
        .and_then(serde_json::Value::as_str);
    if focused_kind == Some("field") {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "runtime_tool_paste_secret_to_slice",
        message: "paste_secret_to_slice requires a focused fillable browser field or a selector/field_id returned by slice_browser_find".to_string(),
    })
}

pub(super) fn browser_selector(selector: Option<&str>, field_id: Option<&str>) -> Option<String> {
    selector
        .or(field_id)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn required_browser_selector(
    selector: Option<&str>,
    field_id: Option<&str>,
    operation: &'static str,
) -> Result<String, DaemonError> {
    browser_selector(selector, field_id).ok_or_else(|| DaemonError::LocalTransport {
        operation,
        message: "selector or field_id is required".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_target_expectation_rejects_wrong_host() {
        let status = serde_json::json!({
            "url": "https://example.com/signup",
            "focusedElement": {"kind": "field"},
            "fields": []
        });
        let args = crate::transport::runtime_tools::PasteSecretToSliceArgs {
            credential_id: "demo".to_string(),
            submit: false,
            expected_host: Some("accounts.google.com".to_string()),
            expected_url: None,
            selector: None,
            field_id: None,
        };

        let error = ensure_browser_target_matches_expectations(&status, &args)
            .expect_err("wrong expected_host should fail");

        assert!(error.to_string().contains("does not match expected host"));
    }

    #[test]
    fn browser_fill_target_requires_focused_field_or_selector() {
        let status = serde_json::json!({
            "url": "https://example.com/signup",
            "focusedElement": {"kind": "button"},
            "fields": [{"selector": "#password"}]
        });

        ensure_browser_fill_target(&status, Some("#password"))
            .expect("known fillable selector should pass");
        assert!(ensure_browser_fill_target(&status, None).is_err());
    }
}

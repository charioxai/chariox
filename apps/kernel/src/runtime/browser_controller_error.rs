//! MP-08/MP-10/MP-11: one cause projection for local, slice-peer and home-worker
//! Browser routes. A precise failure never authorizes replaying a mutation.
use crate::error::DaemonError;

pub(crate) fn browser_controller_error_code(error: &DaemonError) -> Option<&str> {
    match error {
        DaemonError::LocalTransport { operation, message }
            if operation.starts_with("browser_controller.") =>
        {
            let code = if let Some(rest) = message.strip_prefix("browser controller `") {
                let (method, rest) = rest.split_once("` failed with ")?;
                if !method.starts_with("browser.") {
                    return None;
                }
                rest.split_once(':')?.0
            } else {
                message.split_once(':')?.0
            };
            is_controller_code(code).then_some(code)
        }
        DaemonError::RelayTransport { code, .. } if is_controller_code(code) => Some(code),
        _ => None,
    }
}

fn is_controller_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 96
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && (code.starts_with("browser_")
            || code.starts_with("snapshot_")
            || code.starts_with("environment_")
            || matches!(code, "stale_element_reference" | "stale_document_reference"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp08_cause_projection_is_scoped_and_rejects_untrusted_text() {
        for (operation, message) in [
            (
                "runtime_tool_read_file",
                "browser_cdp_timeout: user-authored text",
            ),
            (
                "browser_controller.route",
                "user content failed with browser_cdp_timeout: text",
            ),
            (
                "browser_controller.route",
                "browser controller `health` failed with browser_cdp_timeout: text",
            ),
            (
                "browser_controller.route",
                "browser controller `browser.action` failed with invalid code: text",
            ),
        ] {
            assert_eq!(
                browser_controller_error_code(&DaemonError::LocalTransport {
                    operation,
                    message: message.into(),
                }),
                None
            );
        }
        for code in [
            "environment_stale_element_reference",
            "environment_stale_document_revision",
        ] {
            let error = DaemonError::LocalTransport {
                operation: "browser_controller.action",
                message: format!("{code}: fixture cause"),
            };
            assert_eq!(browser_controller_error_code(&error), Some(code));
        }
        for code in ["transport_error", "target_not_connected"] {
            let delivery = DaemonError::RelayTransport {
                operation: "read relay peer response",
                code: code.into(),
                message: "browser_cdp_timeout appears only in untrusted text".into(),
                retryable: true,
            };
            assert_eq!(browser_controller_error_code(&delivery), None);
        }
    }
}

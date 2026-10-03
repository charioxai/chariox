//! Compatibility guidance in existing transport error messages, without changing
//! the request or response contracts. Keep serde's detail for client diagnostics.
pub(crate) fn message(prefix: &str, error: &serde_json::Error) -> String {
    let detail = format!("{prefix}: {error}");
    if !error.is_data() {
        return detail;
    }
    format!(
        "{detail}. This kernel (protocol {}) does not support this request; update the Chariox client or kernel so both match.",
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalDaemonRequest;

    #[test]
    fn request_decode_refusal_names_kernel_and_upgrade_for_unknown_variant_and_field_shapes() {
        for payload in [
            r#"{"FutureRequest":{}}"#,
            r#"{"AcceptAppHostAction":{"session_id":"s","operation_id":"o","future":true}}"#,
            r#"{"AcceptAppHostAction":{"session_id":"s","operation_id":17}}"#,
        ] {
            let error = serde_json::from_str::<LocalDaemonRequest>(payload).unwrap_err();
            let message = message("invalid request", &error);
            assert!(message.starts_with(&format!("invalid request: {error}")));
            assert!(message.ends_with(&format!("This kernel (protocol {}) does not support this request; update the Chariox client or kernel so both match.", crate::local::LOCAL_DAEMON_PROTOCOL_VERSION)));
        }
    }

    #[test]
    fn request_decode_refusal_preserves_syntax_errors_and_supported_requests() {
        let error = serde_json::from_str::<LocalDaemonRequest>("not json").unwrap_err();
        assert_eq!(
            message("invalid request", &error),
            format!("invalid request: {error}")
        );
        assert!(serde_json::from_str::<LocalDaemonRequest>(
            r#"{"AcceptAppHostAction":{"session_id":"s","operation_id":"o"}}"#
        )
        .is_ok());
    }
}

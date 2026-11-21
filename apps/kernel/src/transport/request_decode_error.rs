//! Shape diagnostics shared by the direct and relay request transports.

use serde_json::Value;

/// The prefix of the message for a request that does not parse; the only `invalid_request`
/// text an external grant holder may read.
pub(crate) const DECODE_PREFIX: &str = "invalid request";

pub(crate) fn message(prefix: &str, error: &serde_json::Error, request: Option<&Value>) -> String {
    if let Some(request) = request {
        if let Err(tracked) =
            serde_path_to_error::deserialize::<_, crate::local::LocalDaemonRequest>(request)
        {
            let detail = tracked.inner().to_string();
            let path = tracked.path();
            if path.iter().next().is_none() {
                if let Some(name) = single_variant_name(request) {
                    if detail.starts_with(&format!("unknown variant `{name}`,")) {
                        return format!(
                            "{prefix}: unknown request type `{name}`. This kernel (protocol {}) does not support this request; update the Chariox client or kernel so both match.",
                            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                        );
                    }
                }
            } else {
                return format!("{prefix}: {path}: {detail}");
            }
        }
    }
    format!("{prefix}: {error}")
}

pub(super) fn envelope_detail<T: serde::de::DeserializeOwned>(payload: &Value) -> Option<String> {
    let error = serde_path_to_error::deserialize::<_, T>(payload).err()?;
    let path = error.path();
    if path.iter().next().is_none() {
        Some(error.inner().to_string())
    } else {
        Some(format!("{path}: {}", error.inner()))
    }
}

fn single_variant_name(request: &Value) -> Option<&str> {
    if let Some(name) = request.as_str() {
        return Some(name);
    }
    let fields = request.as_object()?;
    (fields.len() == 1).then(|| fields.keys().next().unwrap().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalDaemonRequest;

    #[test]
    fn request_decode_refusal_distinguishes_unknown_requests_and_bad_fields() {
        for (payload, expected, suggests_upgrade) in [
            (
                r#"{"FutureRequest":{}}"#,
                "unknown request type `FutureRequest`",
                true,
            ),
            (
                r#""FutureRequest""#,
                "unknown request type `FutureRequest`",
                true,
            ),
            (
                r#"{"AcceptAppHostAction":{"session_id":"s","operation_id":"o","future":true}}"#,
                "unknown field `future`",
                false,
            ),
            (
                r#"{"AcceptAppHostAction":{"session_id":"s","operation_id":17}}"#,
                "AcceptAppHostAction.operation_id: invalid type: integer `17`, expected a string",
                false,
            ),
        ] {
            let error = serde_json::from_str::<LocalDaemonRequest>(payload).unwrap_err();
            let request = serde_json::from_str(payload).unwrap();
            let message = message(DECODE_PREFIX, &error, Some(&request));
            assert!(message.contains(expected), "{message}");
            assert_eq!(
                message.contains("update the Chariox client or kernel"),
                suggests_upgrade,
                "{message}"
            );
        }
    }

    #[test]
    fn request_decode_refusal_preserves_syntax_errors() {
        let error = serde_json::from_str::<LocalDaemonRequest>("not json").unwrap_err();
        assert_eq!(
            message(DECODE_PREFIX, &error, None),
            format!("invalid request: {error}")
        );
    }
}

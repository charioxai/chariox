//! MP-08 / MP-10 / MP-11: fixed provider error categories, never error payloads.
use serde_json::Value;

pub(super) fn record(error: &Value) {
    let (category, status) = classify(error);
    crate::logging::warn_with_fields(
        "daemon.opencode_error_category",
        "Official provider error category",
        serde_json::json!({"category": category, "http_status": status}),
    );
}

fn classify(error: &Value) -> (&'static str, Option<u64>) {
    let status = error
        .pointer("/data/statusCode")
        .or_else(|| error.get("statusCode"))
        .and_then(Value::as_u64)
        .filter(|status| (100..600).contains(status));
    let name = error
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = error
        .pointer("/data/message")
        .or_else(|| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let category =
        if status == Some(401) || message.contains("unauthorized") || name == "ProviderAuthError" {
            "authentication"
        } else if status == Some(402)
            || message.contains("insufficient")
            || message.contains("credit")
            || message.contains("usage limit")
            || name == "UsageLimitError"
        {
            "funds_or_usage_limit"
        } else if status == Some(429) || message.contains("rate limit") {
            "rate_limit"
        } else if name == "ContextOverflowError"
            || message.contains("context length")
            || message.contains("context window")
            || message.contains("too many tokens")
        {
            "context_limit"
        } else if name == "ProviderModelNotFoundError"
            || message.contains("model not found")
            || message.contains("unknown model")
        {
            "model_not_found"
        } else if name == "MessageAbortedError" {
            "aborted"
        } else if status == Some(403) {
            "access_denied"
        } else if status == Some(400) {
            "request_rejected"
        } else if status.is_some_and(|status| status >= 500) {
            "provider_service_error"
        } else {
            "unclassified_provider_error"
        };
    (category, status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opencode_diagnostics_publish_only_fixed_categories_and_bounded_status() {
        for (error, expected) in [
            (
                serde_json::json!({"name":"APIError","data":{"statusCode":402,"message":"private provider payload"}}),
                ("funds_or_usage_limit", Some(402)),
            ),
            (
                serde_json::json!({"name":"ContextOverflowError","data":{"message":"private provider payload"}}),
                ("context_limit", None),
            ),
            (
                serde_json::json!({"name":"private provider payload","data":{"statusCode":999,"message":"private provider payload"}}),
                ("unclassified_provider_error", None),
            ),
        ] {
            assert_eq!(classify(&error), expected);
            assert!(!serde_json::to_string(&classify(&error))
                .unwrap()
                .contains("private provider payload"));
        }
    }
}

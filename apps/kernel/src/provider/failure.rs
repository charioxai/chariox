//! EA2-75: why a provider turn failed, typed once from the harness's own error.
//!
//! Chariox reports the cause and retryability, preserving the harness's
//! explicit retryability when it supplies one. It never retries, backs off or counts quota; the provider's
//! native behaviour stays as it is.

use serde::{Deserialize, Serialize};

const DISPATCH_PREFIX: &str = "Provider prompt dispatch failed: ";
const SUBSTITUTABLE_PREFIX: &str = "Provider reported a substitutable resource limit: ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFailureCause {
    Overloaded,
    UsageLimit,
    SignInRequired,
    ProviderCrashed,
    Network,
    Other,
}

impl ProviderFailureCause {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overloaded => "overloaded",
            Self::UsageLimit => "usage limit",
            Self::SignInRequired => "sign-in required",
            Self::ProviderCrashed => "provider crashed",
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFailure {
    pub cause: ProviderFailureCause,
    /// A later attempt may succeed without anyone changing anything.
    pub retryable: bool,
    /// The harness's own error text, with credentials removed.
    pub message: String,
}

impl ProviderFailure {
    /// One line for event summaries, such as `overloaded, retryable`.
    pub fn summary(&self) -> String {
        let retry = if self.retryable {
            "retryable"
        } else {
            "not retryable"
        };
        format!("{}, {retry}", self.cause.label())
    }

    pub(crate) fn new(cause: ProviderFailureCause, retryable: bool, message: &str) -> Self {
        Self {
            cause,
            retryable,
            message: crate::secret_redaction::redact_secrets(message).into_owned(),
        }
    }

    /// Types a harness error from the code its adapter framed (see
    /// `provider_coded_failure_text`), then from the words of the error.
    pub(crate) fn classify(adapter_key: &str, text: &str) -> Self {
        let text = text.strip_prefix(DISPATCH_PREFIX).unwrap_or(text);
        let text = text.strip_prefix(SUBSTITUTABLE_PREFIX).unwrap_or(text);
        if let Some((code, message, native_retryable)) =
            super::prompt_signals::split_coded_failure(text)
        {
            let mut failure = if let Some((cause, retryable)) = failure_for_code(code, message) {
                Self::new(cause, retryable, message)
            } else {
                Self::classify_words(adapter_key, message)
            };
            if let Some(retryable) = native_retryable {
                failure.retryable = retryable;
            }
            return failure;
        }
        Self::classify_words(adapter_key, text)
    }

    fn classify_words(adapter_key: &str, message: &str) -> Self {
        let normalized = message.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|needle| normalized.contains(needle));
        let (cause, retryable) = if reports_overload(&normalized) {
            (ProviderFailureCause::Overloaded, true)
        } else if super::renewal_failure::renewal_failed(adapter_key, message)
            || has(&[
                "please run /login",
                "invalid api key",
                "not logged in",
                "not signed in",
                "log in again",
                "sign in again",
                "login required",
                "authentication required",
                "invalid authentication",
                "unauthorized",
            ])
        {
            (ProviderFailureCause::SignInRequired, false)
        } else if super::prompt_signals::text_reports_resource_limit(message) {
            (
                ProviderFailureCause::UsageLimit,
                rate_limit_clears(&normalized),
            )
        } else if has(&[
            "exited before",
            "exited unexpectedly",
            "provider process exited",
            "terminated by signal",
            "crashed",
        ]) {
            (ProviderFailureCause::ProviderCrashed, true)
        } else if has(&[
            "connection refused",
            "connection reset",
            "connection closed",
            "connection error",
            "connection failed",
            "error sending request",
            "failed to connect",
            "unable to connect",
            "econnrefused",
            "econnreset",
            "enotfound",
            "etimedout",
            "dns error",
            "timed out",
            "network is unreachable",
            "stream disconnected",
            "stream closed before",
            "routing discovery failed",
            "fetch failed",
            "socket hang up",
        ]) {
            (ProviderFailureCause::Network, true)
        } else {
            (ProviderFailureCause::Other, false)
        };
        Self::new(cause, retryable, message)
    }
}

fn reports_overload(normalized: &str) -> bool {
    [
        "overloaded",
        "at capacity",
        "http 529",
        "status 529",
        "api error: 529",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

/// A plain HTTP 429 or per-minute rate limit clears by itself; an exhausted
/// plan or balance does not.
fn rate_limit_clears(normalized: &str) -> bool {
    let transient = [
        "not your usage limit",
        "temporarily limiting",
        "per-minute",
        "per minute",
        "too many requests",
    ];
    let exhausted = [
        "usage limit",
        "session limit",
        "monthly limit",
        "quota",
        "credit",
        "billing",
        "spend limit",
        "balance",
    ];
    transient.iter().any(|word| normalized.contains(word))
        || !exhausted.iter().any(|word| normalized.contains(word))
}

/// Codex `codexErrorInfo` variants (snake case), Claude `StopFailure` kinds and
/// OpenCode error names, by the code each adapter frames.
fn failure_for_code(code: &str, message: &str) -> Option<(ProviderFailureCause, bool)> {
    use ProviderFailureCause::*;
    if let Some(status) = code
        .strip_prefix("http_status_")
        .and_then(|s| s.parse::<u16>().ok())
    {
        if super::renewal_failure::renewal_failed("codex", message) {
            return Some((SignInRequired, false));
        }
        return Some(match status {
            408 | 500..=599 => (Network, true),
            _ => (Other, false),
        });
    }
    Some(match code {
        "server_overloaded" => (Overloaded, true),
        "usage_limit_exceeded" | "billing_error" | "session_budget_exceeded" => (UsageLimit, false),
        "rate_limit_exceeded" | "rate_limit" => {
            (UsageLimit, rate_limit_clears(&message.to_lowercase()))
        }
        "unauthorized"
        | "authentication_failed"
        | "oauth_org_not_allowed"
        | "provider_auth_error" => (SignInRequired, false),
        "http_connection_failed"
        | "response_stream_connection_failed"
        | "response_stream_disconnected" => {
            // Older app-server errors can omit the HTTP status detail.
            if super::renewal_failure::renewal_failed("codex", message) {
                (SignInRequired, false)
            } else {
                (Network, true)
            }
        }
        "internal_server_error" | "server_error" => {
            if reports_overload(&message.to_lowercase()) {
                (Overloaded, true)
            } else {
                (Other, true)
            }
        }
        "context_window_exceeded"
        | "max_output_tokens"
        | "bad_request"
        | "invalid_request"
        | "model_not_found"
        | "cyber_policy"
        | "misalignment_policy_violation"
        | "sandbox_error"
        | "thread_rollback_failed" => (Other, false),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderRunTermination;

    #[test]
    fn process_exits_and_transport_losses_are_typed_without_any_text() {
        for termination in [
            ProviderRunTermination::process_exit(17, 1),
            ProviderRunTermination::signal("SIGKILL", 1),
            ProviderRunTermination::unknown_process_exit(1),
        ] {
            let failure = termination.failure.expect("typed crash");
            assert_eq!(failure.cause, ProviderFailureCause::ProviderCrashed);
            assert!(failure.retryable);
        }
        let failure = ProviderRunTermination::transport_failure("relay closed", 1)
            .failure
            .expect("typed network loss");
        assert_eq!(failure.cause, ProviderFailureCause::Network);
        assert!(failure.retryable);
    }

    #[test]
    fn the_harness_text_stays_whole_except_for_credentials() {
        let failure = ProviderFailure::classify(
            "claude",
            "Provider prompt dispatch failed: Claude StopFailure [server_error]: \
             API Error: 500 request failed for Authorization: Bearer sk-ant-abcdefghijklmnop0123456789",
        );
        assert_eq!(failure.cause, ProviderFailureCause::Other);
        assert!(failure.retryable);
        assert!(failure
            .message
            .starts_with("API Error: 500 request failed for"));
        assert!(
            !failure.message.contains("abcdefghijklmnop"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn native_failure_text_keeps_whitespace_and_is_not_truncated() {
        let message = format!("  First line\n{}\n  ", "native diagnostic ".repeat(200));
        let framed =
            super::super::prompt_signals::provider_coded_failure_text("Codex", "other", &message);
        assert_eq!(ProviderFailure::classify("codex", &framed).message, message);
    }

    #[test]
    fn an_unrecognised_failure_is_other_and_not_retryable() {
        let failure = ProviderFailure::classify("codex", "Codex error [other]: something odd");
        assert_eq!(failure.cause, ProviderFailureCause::Other);
        assert!(!failure.retryable);
        assert_eq!(failure.message, "something odd");
    }
}

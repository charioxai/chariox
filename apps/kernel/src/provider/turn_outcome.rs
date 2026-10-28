//! How a turn ended, typed once: the outcome every client reads from the turn's
//! event, history and state.
//!
//! A failed turn gets its kind from the harness's own error (EA2-75). Chariox
//! reports the kind and retryability, preserving the harness's explicit
//! retryability when it supplies one. It never retries, backs off or counts quota; the provider's
//! native behaviour stays as it is. A reader decides to retry or stop from `kind` and
//! `retryable` only; `reason` is for people.

use serde::{Deserialize, Serialize};

const DISPATCH_PREFIX: &str = "Provider prompt dispatch failed: ";
const SUBSTITUTABLE_PREFIX: &str = "Provider reported a substitutable resource limit: ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcomeKind {
    Completed,
    Cancelled,
    /// The kernel restarted while the turn ran and could not resume it.
    InterruptedByRestart,
    Overloaded,
    UsageLimit,
    SignInRequired,
    ProviderCrashed,
    Network,
    Other,
    /// A kind a newer kernel added; `reason` still says what happened.
    #[serde(other)]
    Unknown,
}

impl TurnOutcomeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::InterruptedByRestart => "interrupted by restart",
            Self::Unknown => "unknown",
            Self::Overloaded => "overloaded",
            Self::UsageLimit => "usage limit",
            Self::SignInRequired => "Sign-in required",
            Self::ProviderCrashed => "provider crashed",
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOutcome {
    pub kind: TurnOutcomeKind,
    /// A later attempt may succeed without anyone changing anything.
    pub retryable: bool,
    /// For people: the harness's own error text, with credentials removed.
    pub reason: String,
}

impl TurnOutcomeKind {
    fn headline(self) -> &'static str {
        match self {
            Self::Completed => "Turn completed",
            Self::Cancelled => "Turn cancelled",
            Self::InterruptedByRestart => "Interrupted by a restart",
            Self::Unknown => "Turn ended",
            Self::Overloaded => "Provider overloaded",
            Self::UsageLimit => "Usage limit reached",
            Self::SignInRequired => "Sign-in required",
            Self::ProviderCrashed => "Provider stopped unexpectedly",
            Self::Network => "Network problem",
            Self::Other => "Provider error",
        }
    }
}

impl TurnOutcome {
    /// One line for event summaries, such as `overloaded, retryable`.
    pub fn summary(&self) -> String {
        let retry = if self.retryable {
            "retryable"
        } else {
            "not retryable"
        };
        format!("{}, {retry}", self.kind.label())
    }

    pub fn completed() -> Self {
        Self::of(TurnOutcomeKind::Completed, false, "")
    }

    pub fn cancelled() -> Self {
        Self::of(TurnOutcomeKind::Cancelled, false, "")
    }

    /// The kernel restarted and could not resume the turn; sending the request
    /// again continues the same conversation.
    pub fn interrupted_by_restart(reason: &str) -> Self {
        Self::new(TurnOutcomeKind::InterruptedByRestart, true, reason)
    }

    /// The outcome of a settled turn: its status, and for a failed turn the
    /// failure the provider reported.
    pub(crate) fn settled(
        status: crate::git_observer::CompletedTurnSettlementStatus,
        failure: Option<Self>,
    ) -> Self {
        use crate::git_observer::CompletedTurnSettlementStatus as Status;
        match status {
            Status::Completed => Self::completed(),
            Status::Cancelled => Self::cancelled(),
            Status::Failed => failure.unwrap_or_else(|| Self::of(TurnOutcomeKind::Other, false, "")),
        }
    }

    /// `completed` and `cancelled` are how a turn ends, not errors.
    pub fn is_error(&self) -> bool {
        !matches!(
            self.kind,
            TurnOutcomeKind::Completed | TurnOutcomeKind::Cancelled
        )
    }

    fn of(kind: TurnOutcomeKind, retryable: bool, reason: &str) -> Self {
        Self {
            kind,
            retryable,
            reason: reason.to_string(),
        }
    }

    /// What the user does next. Chariox retries nothing, so this only says
    /// what to change before sending the request again.
    fn recovery(&self) -> &'static str {
        match (self.kind, self.retryable) {
            (TurnOutcomeKind::Completed | TurnOutcomeKind::Cancelled, _) => "",
            (TurnOutcomeKind::InterruptedByRestart, _) => {
                "The kernel restarted while this turn was running; send the request again and the agent continues the same conversation."
            }
            (TurnOutcomeKind::Overloaded, _) => {
                "The provider is overloaded; send the request again in a moment."
            }
            (TurnOutcomeKind::UsageLimit, true) => {
                "The provider is rate limiting requests; wait a minute, then send the request again."
            }
            (TurnOutcomeKind::UsageLimit, false) => {
                "Wait for the limit to reset (the message gives the time when the provider does) or switch this agent to another account, then send the request again."
            }
            (TurnOutcomeKind::SignInRequired, _) => {
                "Sign in again in Provider Accounts, then send the request again."
            }
            (TurnOutcomeKind::ProviderCrashed, _) => {
                "The provider process ended; send the request again to start it."
            }
            (TurnOutcomeKind::Network, _) => {
                "Check the connection to the provider, then send the request again."
            }
            (TurnOutcomeKind::Other | TurnOutcomeKind::Unknown, true) => "Send the request again.",
            (TurnOutcomeKind::Other | TurnOutcomeKind::Unknown, false) => {
                "Fix the cause named in the message, then send the request again."
            }
        }
    }

    /// The transcript line every client shows for a failed turn: the cause,
    /// the harness's own words and what to do next.
    pub fn notice(&self) -> String {
        const MAX_MESSAGE_CHARS: usize = 400;
        let message = self
            .reason
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let message = if message.chars().count() > MAX_MESSAGE_CHARS {
            format!(
                "{}...",
                message.chars().take(MAX_MESSAGE_CHARS).collect::<String>()
            )
        } else {
            message
        };
        let message = message.trim_end_matches('.');
        let headline = self.kind.headline();
        let reason = if message.is_empty() {
            String::new()
        } else {
            format!(" {message}.")
        };
        format!("{headline}.{reason} {}", self.recovery())
    }

    pub(crate) fn new(kind: TurnOutcomeKind, retryable: bool, reason: &str) -> Self {
        Self::of(
            kind,
            retryable,
            &crate::secret_redaction::redact_secrets(reason),
        )
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
            (TurnOutcomeKind::Overloaded, true)
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
            (TurnOutcomeKind::SignInRequired, false)
        } else if super::prompt_signals::text_reports_resource_limit(message) {
            (
                TurnOutcomeKind::UsageLimit,
                rate_limit_clears(&normalized),
            )
        } else if has(&[
            "exited before",
            "exited unexpectedly",
            "provider process exited",
            "terminated by signal",
            "crashed",
        ]) {
            (TurnOutcomeKind::ProviderCrashed, true)
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
            (TurnOutcomeKind::Network, true)
        } else {
            (TurnOutcomeKind::Other, false)
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
        "hit your",
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
fn failure_for_code(code: &str, message: &str) -> Option<(TurnOutcomeKind, bool)> {
    use TurnOutcomeKind::*;
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
            assert_eq!(failure.kind, TurnOutcomeKind::ProviderCrashed);
            assert!(failure.retryable);
        }
        let failure = ProviderRunTermination::transport_failure("relay closed", 1)
            .failure
            .expect("typed network loss");
        assert_eq!(failure.kind, TurnOutcomeKind::Network);
        assert!(failure.retryable);
    }

    #[test]
    fn the_harness_text_stays_whole_except_for_credentials() {
        let failure = TurnOutcome::classify(
            "claude",
            "Provider prompt dispatch failed: Claude StopFailure [server_error]: \
             API Error: 500 request failed for Authorization: Bearer sk-ant-abcdefghijklmnop0123456789",
        );
        assert_eq!(failure.kind, TurnOutcomeKind::Other);
        assert!(failure.retryable);
        assert!(failure
            .reason
            .starts_with("API Error: 500 request failed for"));
        assert!(
            !failure.reason.contains("abcdefghijklmnop"),
            "{}",
            failure.reason
        );
    }

    #[test]
    fn native_failure_text_keeps_whitespace_and_is_not_truncated() {
        let message = format!("  First line\n{}\n  ", "native diagnostic ".repeat(200));
        let framed =
            super::super::prompt_signals::provider_coded_failure_text("Codex", "other", &message);
        assert_eq!(TurnOutcome::classify("codex", &framed).reason, message);
    }

    #[test]
    fn every_class_notice_names_the_cause_the_harness_words_and_a_recovery() {
        let weekly = TurnOutcome::classify(
            "claude",
            "You've hit your weekly limit \u{b7} resets Oct 28, 9pm (UTC)",
        );
        assert_eq!(weekly.kind, TurnOutcomeKind::UsageLimit);
        assert!(!weekly.retryable);
        assert_eq!(
            weekly.notice(),
            "Usage limit reached. You've hit your weekly limit \u{b7} resets Oct 28, 9pm (UTC). Wait for the limit to reset (the message gives the time when the provider does) or switch this agent to another account, then send the request again."
        );
        for (adapter, text, headline, recovery) in [
            (
                "claude",
                "API Error: 529 Overloaded. This is a server-side issue",
                "Provider overloaded.",
                "send the request again in a moment",
            ),
            (
                "claude",
                "Not logged in \u{b7} Please run /login",
                "Sign-in required.",
                "Sign in again in Provider Accounts",
            ),
            (
                "claude",
                "API Error: Connection refused (ECONNREFUSED)",
                "Network problem.",
                "Check the connection to the provider",
            ),
            (
                "claude",
                "Claude Code exited before completing the active turn: exit status: 1",
                "Provider stopped unexpectedly.",
                "send the request again to start it",
            ),
        ] {
            let notice = TurnOutcome::classify(adapter, text).notice();
            assert!(notice.starts_with(headline), "{notice}");
            assert!(notice.contains(text.trim_end_matches('.')), "{notice}");
            assert!(notice.contains(recovery), "{notice}");
        }
    }

    #[test]
    fn an_unrecognised_failure_is_other_and_not_retryable() {
        let failure = TurnOutcome::classify("codex", "Codex error [other]: something odd");
        assert_eq!(failure.kind, TurnOutcomeKind::Other);
        assert!(!failure.retryable);
        assert_eq!(failure.reason, "something odd");
    }
}

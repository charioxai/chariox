//! How a turn ended, typed once: the outcome every client reads from the turn's
//! event, history and state.
//!
//! A failed turn gets its kind from the harness's own error (EA2-75). Chariox
//! reports the kind and retryability, preserving the harness's explicit
//! retryability when it supplies one. Native diagnostics go to the kernel log.
//! It never retries, backs off or tracks usage limits; the provider's
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

// Match the key order of projected JSON snapshots as well as direct replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOutcome {
    pub kind: TurnOutcomeKind,
    /// A calm explanation for people. Native provider diagnostics stay in the log.
    pub reason: String,
    /// A later attempt may succeed without anyone changing anything.
    pub retryable: bool,
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
            Status::Failed => {
                failure.unwrap_or_else(|| Self::of(TurnOutcomeKind::Other, false, ""))
            }
        }
    }

    /// `completed` and `cancelled` are how a turn ends, not errors.
    pub fn is_error(&self) -> bool {
        !matches!(
            self.kind,
            TurnOutcomeKind::Completed | TurnOutcomeKind::Cancelled
        )
    }

    /// How a turn that ends with this outcome is settled: a turn a person
    /// stopped is cancelled, not failed.
    pub(crate) fn settlement_status(&self) -> crate::git_observer::CompletedTurnSettlementStatus {
        if self.is_error() {
            crate::git_observer::CompletedTurnSettlementStatus::Failed
        } else {
            crate::git_observer::CompletedTurnSettlementStatus::Cancelled
        }
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
            (TurnOutcomeKind::Overloaded, true) => {
                "Try again in a moment or choose another model."
            }
            (TurnOutcomeKind::Overloaded, false) => {
                "Choose another model, then send the request again."
            }
            (TurnOutcomeKind::UsageLimit, true) => {
                "The provider is rate limiting requests; wait a minute, then send the request again."
            }
            (TurnOutcomeKind::UsageLimit, false) => {
                "Wait for the limit to reset or choose another account, then send the request again."
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
                "Check the provider account and model settings, then send the request again."
            }
        }
    }

    /// The transcript line every client shows. Adapter diagnostics enter only
    /// through classification; kernel-authored reasons can name a missing workspace.
    pub fn notice(&self) -> String {
        let reason = if self.reason.is_empty() {
            format!("{}.", self.kind.headline())
        } else {
            self.reason.clone()
        };
        format!("{reason} {}", self.recovery())
            .trim_end()
            .to_string()
    }

    /// A kernel-authored explanation, such as a removed workspace or worker.
    pub(crate) fn new(kind: TurnOutcomeKind, retryable: bool, reason: &str) -> Self {
        Self::of(
            kind,
            retryable,
            &crate::secret_redaction::redact_secrets(reason),
        )
    }

    /// Provider diagnostics stay in logs except for Claude's bounded usage-limit detail.
    pub(crate) fn provider_failure(kind: TurnOutcomeKind, retryable: bool) -> Self {
        let reason = match kind {
            TurnOutcomeKind::Completed | TurnOutcomeKind::Cancelled => "",
            TurnOutcomeKind::InterruptedByRestart => "The kernel restarted during this turn.",
            TurnOutcomeKind::Overloaded => "The provider is at capacity.",
            TurnOutcomeKind::UsageLimit => "The provider's usage limit was reached.",
            TurnOutcomeKind::SignInRequired => "The provider needs you to sign in.",
            TurnOutcomeKind::ProviderCrashed => "The provider process stopped unexpectedly.",
            TurnOutcomeKind::Network => "The connection to the provider was interrupted.",
            TurnOutcomeKind::Other | TurnOutcomeKind::Unknown => {
                "The provider could not complete this request."
            }
        };
        Self::of(kind, retryable, reason)
    }

    /// Types a harness error from the code its adapter framed (see
    /// `provider_coded_failure_text`), then from the words of the error.
    pub(crate) fn classify(adapter_key: &str, text: &str) -> Self {
        crate::logging::info_with_fields(
            "daemon.provider",
            "provider reported a turn failure",
            serde_json::json!({
                "adapter_key": adapter_key,
                "diagnostic": crate::secret_redaction::redact_secrets(text),
            }),
        );
        let text = text.strip_prefix(DISPATCH_PREFIX).unwrap_or(text);
        let text = text.strip_prefix(SUBSTITUTABLE_PREFIX).unwrap_or(text);
        if let Some((code, message, native_retryable)) =
            super::prompt_signals::split_coded_failure(text)
        {
            let mut failure = if let Some((cause, retryable)) = failure_for_code(code, message) {
                Self::provider_failure_for_message(adapter_key, cause, retryable, message)
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
            (TurnOutcomeKind::UsageLimit, rate_limit_clears(&normalized))
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
            "waiting for network",
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
        Self::provider_failure_for_message(adapter_key, cause, retryable, message)
    }

    fn provider_failure_for_message(
        adapter_key: &str,
        cause: TurnOutcomeKind,
        retryable: bool,
        message: &str,
    ) -> Self {
        let mut failure = Self::provider_failure(cause, retryable);
        if adapter_key != "claude" || cause != TurnOutcomeKind::UsageLimit {
            return failure;
        }
        if let Some(line) = message.lines().map(str::trim).find(|line| !line.is_empty()) {
            failure.reason = crate::secret_redaction::redact_secrets(line)
                .chars()
                .take(512)
                .collect();
        }
        failure
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
        "server_overloaded" | "flex_unavailable" | "policy_capacity_unavailable" => {
            (Overloaded, true)
        }
        "usage_limit_exceeded" | "billing_error" | "session_budget_exceeded" => (UsageLimit, false),
        "message_aborted" => (Cancelled, false),
        "rate_limit_exceeded" | "rate_limit" => {
            (UsageLimit, rate_limit_clears(&message.to_lowercase()))
        }
        "unauthorized"
        | "authentication_failed"
        | "oauth_org_not_allowed"
        | "provider_auth_error" => (SignInRequired, false),
        "http_connection_failed"
        | "response_stream_connection_failed"
        | "response_stream_disconnected"
        | "response_too_many_failed_attempts" => {
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
        | "invalid_prompt"
        | "too_many_denials"
        | "active_turn_not_steerable"
        | "bio_policy"
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
    fn provider_failure_outcomes_do_not_echo_full_diagnostics() {
        for (adapter, text, kind, retryable) in [
            ("codex", "Codex error [server_overloaded]: Selected model is at capacity. Please try a different model.", TurnOutcomeKind::Overloaded, true),
            ("claude", "Claude StopFailure [rate_limit]: You've hit your weekly limit · resets Oct 28, 9pm (UTC)", TurnOutcomeKind::UsageLimit, false),
            ("claude", "Not logged in · Please run /login", TurnOutcomeKind::SignInRequired, false),
            ("claude", "API Error: Connection refused (ECONNREFUSED)", TurnOutcomeKind::Network, true),
            ("codex", "Codex error [other]: something odd", TurnOutcomeKind::Other, false),
        ] {
            let outcome = TurnOutcome::classify(adapter, text);
            assert_eq!((outcome.kind, outcome.retryable), (kind, retryable));
            assert!(!outcome.reason.contains(text));
            assert!(!outcome.notice().contains(text));
            assert!(outcome.notice().contains("again"));
        }
        let diagnostic = "API Error: 500 request failed for Authorization: Bearer sk-ant-abcdefghijklmnop0123456789";
        let outcome = TurnOutcome::classify("claude", diagnostic);
        assert_eq!(
            outcome.reason,
            "The provider could not complete this request."
        );
        assert!(!outcome.notice().contains("API Error"));
    }

    #[test]
    fn ea2_75_native_diagnostics_reach_the_kernel_log_with_secrets_redacted() {
        let capture = crate::logging::capture::start();
        let diagnostic = "Codex error [server_overloaded]: Selected model is at capacity. Please try a different model.";
        let outcome = TurnOutcome::classify("codex", diagnostic);
        assert!(capture
            .records()
            .iter()
            .any(|record| record.contains(diagnostic)));
        assert!(!outcome.notice().contains(diagnostic));
        let secret = "sk-ant-abcdefghijklmnop0123456789";
        TurnOutcome::classify("claude", &format!("Claude StopFailure [server_error]: API Error: 500 for Authorization: Bearer {secret}"));
        let records = capture.records();
        assert!(records
            .iter()
            .any(|record| record.contains("API Error: 500")));
        assert!(records.iter().all(|record| !record.contains(secret)));
    }

    #[test]
    fn provider_failure_notice_has_an_explanation_and_recovery() {
        let outcome = TurnOutcome::provider_failure(TurnOutcomeKind::Overloaded, true);
        assert_eq!(
            outcome.notice(),
            "The provider is at capacity. Try again in a moment or choose another model."
        );
    }

    #[test]
    fn claude_usage_limit_preserves_redacted_reset_detail_for_people() {
        let outcome = TurnOutcome::classify(
            "claude",
            "Claude StopFailure [rate_limit]: You've hit your weekly limit · resets Oct 28, 9pm (UTC)",
        );
        assert_eq!(outcome.kind, TurnOutcomeKind::UsageLimit);
        assert!(!outcome.retryable);
        assert_eq!(
            outcome.reason,
            "You've hit your weekly limit · resets Oct 28, 9pm (UTC)"
        );

        let secret = TurnOutcome::classify(
            "claude",
            "Claude StopFailure [rate_limit]: weekly limit · resets sk-ant-abcdefghijklmnop0123456789",
        );
        assert!(secret.reason.contains("weekly limit"));
        assert!(!secret.notice().contains("sk-ant-"));

        let long = format!(
            "Claude StopFailure [rate_limit]: weekly limit · resets {}",
            "漢字".repeat(600)
        );
        let bounded = TurnOutcome::classify("claude", &long);
        assert_eq!(bounded.reason.chars().count(), 512);
    }
}

//! How a turn ended, typed once: the outcome every client reads from the turn's
//! event, history and state.
//!
//! A failed turn gets its kind from the harness's own error (EA2-75). Chariox
//! reports the kind and retryability, preserving the harness's explicit
//! retryability when it supplies one. The reason carries a bounded, redacted
//! provider message.
//! It never retries or backs off a failed turn itself; a usage-limit stop is
//! recorded and resumed by the kernel (`usage_limit_runtime`), and the provider's
//! native behaviour stays as it is. A reader decides to retry or stop from `kind` and
//! `retryable` only; `reason` is for people.

use serde::{Deserialize, Serialize};

const DISPATCH_PREFIX: &str = "Provider prompt dispatch failed: ";
const SUBSTITUTABLE_PREFIX: &str = "Provider reported a substitutable resource limit: ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcomeKind {
    Completed,
    AnsweredInTurn,
    Cancelled,
    /// The kernel restarted while the turn ran and could not resume it.
    InterruptedByRestart,
    Overloaded,
    UsageLimit,
    SignInRequired,
    ProviderCrashed,
    Network,
    ModelRefused,
    StreamEnded,
    Other,
    /// A kind a newer kernel added; `reason` still says what happened.
    #[serde(other)]
    Unknown,
}

impl TurnOutcomeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::AnsweredInTurn => "answered in another turn",
            Self::Cancelled => "cancelled",
            Self::InterruptedByRestart => "interrupted by restart",
            Self::Unknown => "unknown",
            Self::Overloaded => "overloaded",
            Self::UsageLimit => "usage limit",
            Self::SignInRequired => "Sign-in required",
            Self::ProviderCrashed => "provider crashed",
            Self::Network => "network",
            Self::ModelRefused => "model refused",
            Self::StreamEnded => "stream ended early",
            Self::Other => "other",
        }
    }
}

// Match the key order of projected JSON snapshots as well as direct replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOutcome {
    pub kind: TurnOutcomeKind,
    /// A redacted explanation for people.
    pub reason: String,
    /// A later attempt may succeed without anyone changing anything.
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_in_turn: Option<AnsweredTurnReceipt>,
}

/// The one answering turn, shared by delivery receipts and transcript navigation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnsweredTurnReceipt {
    pub session_id: String,
    pub agent_id: String,
    pub prompt_id: String,
    pub task_id: String,
}

impl TurnOutcomeKind {
    fn headline(self) -> &'static str {
        match self {
            Self::Completed => "Turn completed",
            Self::AnsweredInTurn => "Answered in another turn",
            Self::Cancelled => "Turn cancelled",
            Self::InterruptedByRestart => "Interrupted by a restart",
            Self::Unknown => "Turn ended",
            Self::Overloaded => "Provider overloaded",
            Self::UsageLimit => "Usage limit reached",
            Self::SignInRequired => "Sign-in required",
            Self::ProviderCrashed => "Provider stopped unexpectedly",
            Self::Network => "Network problem",
            Self::ModelRefused => "The model refused the request",
            Self::StreamEnded => "The provider stream ended before the turn completed",
            Self::Other => "Provider error",
        }
    }
}

impl TurnOutcome {
    pub fn answered_in_turn(receipt: AnsweredTurnReceipt) -> Self {
        Self {
            kind: TurnOutcomeKind::AnsweredInTurn,
            reason: String::new(),
            retryable: false,
            answered_in_turn: Some(receipt),
        }
    }
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
            TurnOutcomeKind::Completed
                | TurnOutcomeKind::AnsweredInTurn
                | TurnOutcomeKind::Cancelled
        )
    }

    /// How a turn that ends with this outcome is settled: a turn a person
    /// stopped is cancelled, not failed.
    pub(crate) fn settlement_status(&self) -> crate::git_observer::CompletedTurnSettlementStatus {
        if self.kind == TurnOutcomeKind::AnsweredInTurn {
            crate::git_observer::CompletedTurnSettlementStatus::Completed
        } else if self.is_error() {
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
            answered_in_turn: None,
        }
    }

    /// What the user does next. Chariox retries nothing, so this only says
    /// what to change before sending the request again.
    fn recovery(&self) -> &'static str {
        match (self.kind, self.retryable) {
            (TurnOutcomeKind::Completed | TurnOutcomeKind::AnsweredInTurn | TurnOutcomeKind::Cancelled, _) => "",
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
                "Wait for the limit to reset or choose another model or account, then send the request again."
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
            (TurnOutcomeKind::StreamEnded, _)
            | (TurnOutcomeKind::Other | TurnOutcomeKind::Unknown, true) => "Send the request again.",
            (TurnOutcomeKind::ModelRefused, _) => "Change the request or choose another model.",
            (TurnOutcomeKind::Other | TurnOutcomeKind::Unknown, false) => {
                "Check the provider account and model settings, then send the request again."
            }
        }
    }

    /// The transcript line every client shows. Adapter diagnostics enter only
    /// through classification; kernel-authored reasons can name a missing workspace.
    pub fn notice(&self) -> String {
        self.notice_with(self.recovery())
    }

    /// The line of a usage-limit turn the kernel resumes by itself, in place of asking the person
    /// to send the request again. `reset` is the clock time the resume is due, when the provider
    /// named one.
    pub fn notice_resuming(&self, reset: Option<&str>) -> String {
        match reset {
            Some(at) => self.notice_with(&format!("Resumes automatically at {at}.")),
            None => self.notice_with("Resumes automatically when usage is back."),
        }
    }

    fn notice_with(&self, recovery: &str) -> String {
        let mut reason = if self.reason.is_empty() {
            format!("{}.", self.kind.headline())
        } else {
            self.reason.clone()
        };
        if !recovery.is_empty() && !reason.ends_with(['.', '?', '!']) {
            reason.push('.');
        }
        format!("{reason} {recovery}").trim_end().to_string()
    }

    /// A kernel-authored explanation, such as a removed workspace or worker.
    pub(crate) fn new(kind: TurnOutcomeKind, retryable: bool, reason: &str) -> Self {
        Self::of(
            kind,
            retryable,
            &crate::secret_redaction::redact_secrets(reason),
        )
    }

    /// Use the product explanation when the provider supplied no message.
    pub(crate) fn provider_failure(kind: TurnOutcomeKind, retryable: bool) -> Self {
        let reason = match kind {
            TurnOutcomeKind::Completed
            | TurnOutcomeKind::AnsweredInTurn
            | TurnOutcomeKind::Cancelled => "",
            TurnOutcomeKind::InterruptedByRestart => "The kernel restarted during this turn.",
            TurnOutcomeKind::Overloaded => "The provider is at capacity.",
            TurnOutcomeKind::UsageLimit => "The provider's usage limit was reached.",
            TurnOutcomeKind::SignInRequired => "The provider needs you to sign in.",
            TurnOutcomeKind::ProviderCrashed => "The provider process stopped unexpectedly.",
            TurnOutcomeKind::Network => "The connection to the provider was interrupted.",
            TurnOutcomeKind::ModelRefused => "The model refused the request.",
            TurnOutcomeKind::StreamEnded => "The provider stream ended before the turn completed.",
            TurnOutcomeKind::Other | TurnOutcomeKind::Unknown => {
                "The provider could not complete this request."
            }
        };
        Self::of(kind, retryable, reason)
    }

    /// Types a harness error from the code its adapter framed (see
    /// `provider_coded_failure_text`), then from the words of the error.
    pub(crate) fn classify(adapter_key: &str, text: &str) -> Self {
        let text = text.trim_start();
        let text = text.strip_prefix(DISPATCH_PREFIX).unwrap_or(text);
        let text = text.strip_prefix(SUBSTITUTABLE_PREFIX).unwrap_or(text);
        let text = text
            .strip_prefix("Provider reported a terminal model error: ")
            .unwrap_or(text);
        let text = text
            .strip_prefix("Provider reported a resource limit: ")
            .unwrap_or(text);
        if adapter_key == "opencode" && text == super::opencode_runtime::NATIVE_PERMISSION_DENIED {
            return Self::cancelled();
        }
        let failure = if let Some((code, message, native_retryable)) =
            super::prompt_signals::split_coded_failure(text)
        {
            let mut failure = if let Some((cause, retryable)) = failure_for_code(code, message) {
                Self::provider_failure_for_message(cause, retryable, message)
            } else {
                Self::classify_words(adapter_key, message)
            };
            if let Some(retryable) = native_retryable {
                failure.retryable = retryable;
            }
            failure
        } else {
            Self::classify_words(adapter_key, text)
        };
        crate::logging::info_with_fields(
            "daemon.provider",
            "provider reported a turn failure",
            serde_json::json!({"adapter_key": adapter_key, "kind": failure.kind}),
        );
        failure
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
        } else if model_refusal(&normalized) {
            (TurnOutcomeKind::ModelRefused, false)
        } else if has(&[
            "stream ended before",
            "closed its stream before",
            "stream ended early",
            "stream ended without",
        ]) {
            (TurnOutcomeKind::StreamEnded, true)
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
        Self::provider_failure_for_message(cause, retryable, message)
    }

    fn provider_failure_for_message(
        cause: TurnOutcomeKind,
        retryable: bool,
        message: &str,
    ) -> Self {
        if matches!(
            cause,
            TurnOutcomeKind::Completed | TurnOutcomeKind::Cancelled
        ) {
            return Self::provider_failure(cause, retryable);
        }
        let reason = match message.trim() {
            "Codex turn failed"
            | "Codex reported an unknown error"
            | "Provider ended the turn with an API error"
            | "provider reported an explicit error" => String::new(),
            message => super::termination::sanitize_provider_failure_message(message),
        };
        if reason.is_empty() {
            Self::provider_failure(cause, retryable)
        } else {
            Self::of(cause, retryable, &reason)
        }
    }
}

fn model_refusal(normalized: &str) -> bool {
    [
        "model refused",
        "unsupported model",
        "invalid model",
        "model access denied",
        "model_not_found",
        "model not found",
        "model does not exist",
        "model is not supported",
        "model rejected",
        "terminal model error",
        "blocked by policy",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
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
            400 | 403 if model_refusal(&message.to_lowercase()) => (ModelRefused, false),
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
        "model_refused"
        | "model_not_found"
        | "too_many_denials"
        | "bio_policy"
        | "cyber_policy"
        | "misalignment_policy_violation" => (ModelRefused, false),
        "stream_ended_early" => (StreamEnded, true),
        "context_window_exceeded"
        | "invalid_prompt"
        | "active_turn_not_steerable"
        | "max_output_tokens"
        | "bad_request"
        | "invalid_request"
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
        // A known exit status is named in the outcome; it is the one crash path that does.
        let exited = ProviderRunTermination::process_exit(17, 1)
            .failure
            .expect("typed crash");
        assert_eq!(exited.kind, TurnOutcomeKind::ProviderCrashed);
        assert!(exited.retryable);
        assert_eq!(exited.reason, "Provider CLI exited with status 17.");
        for termination in [
            ProviderRunTermination::signal("SIGKILL", 1),
            ProviderRunTermination::unknown_process_exit(1),
        ] {
            let failure = termination.failure.expect("typed crash");
            assert_eq!(failure.kind, TurnOutcomeKind::ProviderCrashed);
            assert!(failure.retryable);
            // The exit status and signal stay in the termination record and the log.
            assert_eq!(failure.reason, "The provider process stopped unexpectedly.");
        }
        // A lost host or an unobserved exit carries a written explanation, which the person and
        // the parent agent need: only a raw exit status or signal is replaced by the calm text.
        let lost = ProviderRunTermination::host_lost("The machine hosting the run was lost", 1)
            .failure
            .expect("typed crash");
        assert_eq!(lost.kind, TurnOutcomeKind::ProviderCrashed);
        assert_eq!(lost.reason, "The machine hosting the run was lost");
        let failure = ProviderRunTermination::transport_failure("relay closed", 1)
            .failure
            .expect("typed network loss");
        assert_eq!(failure.kind, TurnOutcomeKind::Network);
        assert!(failure.retryable);
    }

    #[test]
    fn provider_failure_outcomes_keep_bounded_redacted_details() {
        for (text, kind, retryable, reason) in [
            (
                "Codex error [server_overloaded]: Selected model is at capacity.",
                TurnOutcomeKind::Overloaded,
                true,
                "Selected model is at capacity.",
            ),
            (
                "Codex error [unauthorized]: The session expired.",
                TurnOutcomeKind::SignInRequired,
                false,
                "The session expired.",
            ),
            (
                "Codex error [stream_ended_early]: Upstream closed the response.",
                TurnOutcomeKind::StreamEnded,
                true,
                "Upstream closed the response.",
            ),
            (
                "Codex error [other]: Something odd happened.",
                TurnOutcomeKind::Other,
                false,
                "Something odd happened.",
            ),
            (
                "Codex error [other]: ",
                TurnOutcomeKind::Other,
                false,
                "The provider could not complete this request.",
            ),
        ] {
            let outcome = TurnOutcome::classify("codex", text);
            assert_eq!(
                (outcome.kind, outcome.retryable, outcome.reason.as_str()),
                (kind, retryable, reason)
            );
        }
        let secret = "sk-ant-abcdefghijklmnop0123456789";
        let capture = crate::logging::capture::start();
        let diagnostic =
            format!("API Error: 500 request failed for Authorization: Bearer {secret}");
        let outcome = TurnOutcome::classify("claude", &diagnostic);
        assert!(outcome.reason.contains("API Error: 500"));
        assert!(outcome.reason.contains("[redacted]"));
        assert!(!outcome.notice().contains(secret));
        let records = capture.records();
        assert!(records
            .iter()
            .any(|record| record.contains("provider reported a turn failure")));
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

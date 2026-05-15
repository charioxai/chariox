use crate::terminal::TerminalOutputKind;

use super::launch_contract::ProviderResumeState;
use super::runtime_run::ProviderRunTokenUsage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPromptChunk {
    pub kind: TerminalOutputKind,
    pub merge_key: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAssistantCompletion {
    pub message_id: String,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderPromptSignalBatch {
    pub chunks: Vec<ProviderPromptChunk>,
    pub completions: Vec<ProviderAssistantCompletion>,
    pub prompt_completed: bool,
    pub terminal_failure: Option<String>,
    pub notices: Vec<String>,
    pub resolved_model: Option<String>,
    pub resolved_model_source: Option<&'static str>,
    pub resolved_variant: Option<String>,
    pub resolved_usage_tokens_total: Option<u64>,
    pub resolved_usage: Option<ProviderRunTokenUsage>,
    pub resolved_resume_state: Option<ProviderResumeState>,
}

pub(crate) fn classify_provider_terminal_failure_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    if !matches!(adapter_key, "codex" | "opencode") {
        return None;
    }
    if let Some(failure) = classify_provider_substitutable_failure_text(adapter_key, text) {
        return Some(failure);
    }
    let normalized = text.to_lowercase();
    let fatal_model_error = normalized.contains("unsupported model")
        || normalized.contains("invalid model")
        || normalized.contains("model_not_found")
        || normalized.contains("model not found")
        || (normalized.contains("model") && normalized.contains("does not exist"))
        || (normalized.contains("model") && normalized.contains("not supported"))
        || (normalized.contains("model")
            && (normalized.contains("http 400")
                || normalized.contains("status 400")
                || normalized.contains("400 bad request")));
    if !fatal_model_error {
        return None;
    }
    Some(format!(
        "Provider reported a terminal model error: {}",
        compact_provider_error_snippet(text)
    ))
}

pub(crate) fn classify_provider_substitutable_failure_text(
    adapter_key: &str,
    text: &str,
) -> Option<String> {
    if !matches!(adapter_key, "codex" | "opencode") {
        return None;
    }
    let normalized = text.to_lowercase();
    let quota_or_billing = normalized.contains("insufficient_quota")
        || normalized.contains("quota exceeded")
        || normalized.contains("exceeded your current quota")
        || normalized.contains("billing hard limit")
        || normalized.contains("billing limit")
        || normalized.contains("spend limit")
        || normalized.contains("usage limit")
        || normalized.contains("monthly limit")
        || normalized.contains("no credits")
        || normalized.contains("not enough credits")
        || normalized.contains("credits exhausted")
        || normalized.contains("credit balance")
        || normalized.contains("out of credits");
    let rate_or_run_limit = normalized.contains("rate_limit_exceeded")
        || normalized.contains("rate limit exceeded")
        || normalized.contains("rate limited")
        || normalized.contains("too many requests")
        || normalized.contains("http 429")
        || normalized.contains("status 429")
        || normalized.contains("429 too many requests")
        || normalized.contains("run limit")
        || normalized.contains("runs limit")
        || normalized.contains("turn limit");
    if !(quota_or_billing || rate_or_run_limit) {
        return None;
    }
    Some(format!(
        "Provider reported a substitutable resource limit: {}",
        compact_provider_error_snippet(text)
    ))
}

fn compact_provider_error_snippet(text: &str) -> String {
    let mut snippet = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 500;
    if snippet.chars().count() > MAX_CHARS {
        snippet = snippet.chars().take(MAX_CHARS).collect::<String>();
        snippet.push_str("...");
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::{
        classify_provider_substitutable_failure_text, classify_provider_terminal_failure_text,
    };

    #[test]
    fn classifier_detects_provider_model_rejection_text() {
        let failure = classify_provider_terminal_failure_text(
            "codex",
            "Error: HTTP 400 Bad Request: unsupported model gpt-5.2-codex",
        )
        .expect("model rejection text should be classified");

        assert!(failure.contains("terminal model error"));
        assert!(failure.contains("gpt-5.2-codex"));
    }

    #[test]
    fn classifier_ignores_non_provider_text() {
        assert!(classify_provider_terminal_failure_text(
            "dev-stub",
            "unsupported model gpt-5.2-codex"
        )
        .is_none());
        assert!(
            classify_provider_terminal_failure_text("codex", "normal assistant output").is_none()
        );
    }

    #[test]
    fn substitute_classifier_detects_shared_quota_and_limit_errors() {
        let codex_failure = classify_provider_substitutable_failure_text(
            "codex",
            "Error: insufficient_quota: You exceeded your current quota.",
        )
        .expect("codex quota error should be substitutable");
        assert!(codex_failure.contains("substitutable resource limit"));

        let opencode_failure = classify_provider_substitutable_failure_text(
            "opencode",
            "OpenCode error: No credits available for this account",
        )
        .expect("opencode credit error should be substitutable");
        assert!(opencode_failure.contains("No credits"));
    }

    #[test]
    fn substitute_classifier_ignores_model_auth_and_network_errors() {
        assert!(classify_provider_substitutable_failure_text(
            "codex",
            "HTTP 400 Bad Request: unsupported model gpt-5.2-codex"
        )
        .is_none());
        assert!(classify_provider_substitutable_failure_text(
            "opencode",
            "Authentication required. Please login."
        )
        .is_none());
        assert!(
            classify_provider_substitutable_failure_text("codex", "connection refused").is_none()
        );
    }
}

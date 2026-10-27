//! Requests whose turn failed before completing (provider error, limit or
//! crash). A failed request is dropped: the provider may still hold it
//! unanswered in the session it resumes, so the next turn delivered to the
//! agent carries a one-time hidden note telling the model not to act on it
//! unless the user asks again. The note is kept on the agent record, so it
//! survives a kernel restart, and is cleared once a provider accepted it.

use serde::{Deserialize, Serialize};

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;

use super::AgentServiceStore;

/// Only the newest failures matter to the model; older ones were noted before.
const MAX_PENDING_FAILED_REQUESTS: usize = 3;
const EXCERPT_CHARS: usize = 80;
const REASON_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedRequest {
    pub prompt_id: String,
    pub excerpt: String,
    pub reason: String,
}

impl FailedRequest {
    pub(crate) fn new(prompt_id: &str, prompt: &str, reason: String) -> Self {
        Self {
            prompt_id: prompt_id.to_string(),
            excerpt: bounded_single_line(prompt, EXCERPT_CHARS),
            reason,
        }
    }
}

/// A short reason for the failure notice and the note: provider rate, usage
/// and billing limits read "usage limit reached"; anything else is the
/// sanitized provider diagnostic.
pub(crate) fn failed_request_reason(adapter_key: &str, failure_message: &str) -> String {
    if crate::provider::classify_provider_substitutable_failure_text(adapter_key, failure_message)
        .is_some()
    {
        return "usage limit reached".to_string();
    }
    let detail = crate::provider::sanitize_provider_diagnostic(failure_message);
    let detail = detail.trim();
    let detail = detail
        .strip_prefix("Provider prompt dispatch failed: ")
        .unwrap_or(detail)
        .trim_end_matches('.');
    if detail.trim().is_empty() {
        return "provider error".to_string();
    }
    bounded_single_line(detail, REASON_CHARS)
}

/// The transcript entry marking a failed turn, shown on every surface.
pub(crate) fn failed_request_notice(reason: &str) -> String {
    format!("Request not carried out: {reason}. It was dropped; send it again to retry.")
}

fn failed_requests_note(failed_requests: &[FailedRequest]) -> String {
    match failed_requests {
        [] => String::new(),
        [failed] => format!(
            "Your previous request (\"{}\") failed ({}) and was not carried out. Do not act on it unless the user asks again; answer only the current request.",
            failed.excerpt, failed.reason
        ),
        failed_requests => format!(
            "Your previous requests ({}) failed and were not carried out. Do not act on them unless the user asks again; answer only the current request.",
            failed_requests
                .iter()
                .map(|failed| format!("\"{}\" ({})", failed.excerpt, failed.reason))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    }
}

fn bounded_single_line(text: &str, max_chars: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max_chars {
        return line;
    }
    let mut bounded = line.chars().take(max_chars).collect::<String>();
    bounded.push('…');
    bounded
}

impl AgentServiceStore {
    /// Remembers a failed request for the agent's next turn.
    pub(crate) fn record_failed_request_durably(
        &self,
        durable_state_store: &DurableKernelStateStore,
        agent_id: &str,
        failed: FailedRequest,
    ) -> Result<(), DaemonError> {
        self.update_failed_requests_durably(
            durable_state_store,
            agent_id,
            "failed_request_recorded",
            |pending| {
                pending.retain(|pending| pending.prompt_id != failed.prompt_id);
                pending.push(failed);
                let overflow = pending.len().saturating_sub(MAX_PENDING_FAILED_REQUESTS);
                pending.drain(..overflow);
            },
        )
    }

    /// `hidden_system_context` plus the one-time note about the agent's failed
    /// requests, for the next turn delivered to its provider.
    pub(crate) fn hidden_context_with_failed_requests(
        &self,
        agent_id: &str,
        hidden_system_context: &str,
    ) -> String {
        let note = self
            .read()
            .get_agent(agent_id)
            .map(|agent| failed_requests_note(agent.failed_requests()))
            .unwrap_or_default();
        match (hidden_system_context.trim(), note.as_str()) {
            (_, "") => hidden_system_context.to_string(),
            ("", note) => note.to_string(),
            (hidden, note) => format!("{hidden}\n\n{note}"),
        }
    }

    /// Clears the note once the provider accepted `delivered_prompt_id`, the
    /// turn that carried it. Only the active prompt can fail, so a note about
    /// the delivered prompt itself is newer than the delivery and is kept.
    pub(crate) fn consume_failed_requests_durably(
        &self,
        durable_state_store: &DurableKernelStateStore,
        agent_id: &str,
        delivered_prompt_id: &str,
    ) -> Result<(), DaemonError> {
        self.update_failed_requests_durably(
            durable_state_store,
            agent_id,
            "failed_request_note_delivered",
            |pending| pending.retain(|pending| pending.prompt_id == delivered_prompt_id),
        )
    }

    fn update_failed_requests_durably(
        &self,
        durable_state_store: &DurableKernelStateStore,
        agent_id: &str,
        reason: &str,
        update: impl FnOnce(&mut Vec<FailedRequest>),
    ) -> Result<(), DaemonError> {
        let mut agents = self.write();
        let previous = agents.get_agent(agent_id)?;
        let mut failed_requests = previous.failed_requests().to_vec();
        update(&mut failed_requests);
        if failed_requests == previous.failed_requests() {
            return Ok(());
        }
        let updated = agents.set_agent_failed_requests(agent_id, failed_requests)?;
        if let Err(error) = durable_state_store.append_event(
            "agent.updated",
            Some(updated.id().to_string()),
            serde_json::json!({ "agent": &updated, "reason": reason }),
        ) {
            agents.restore_agent(previous);
            return Err(error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(prompt_id: &str, prompt: &str) -> FailedRequest {
        FailedRequest::new(prompt_id, prompt, "usage limit reached".to_string())
    }

    #[test]
    fn provider_limits_read_as_usage_limit_reached() {
        assert_eq!(
            failed_request_reason(
                "claude",
                "Provider prompt dispatch failed: You've hit your session limit · resets 8:30am"
            ),
            "usage limit reached"
        );
        assert_eq!(
            failed_request_reason(
                "claude",
                "Claude StopFailure [rate_limit]: You've hit your session limit · resets 8:30am"
            ),
            "usage limit reached"
        );
        assert_eq!(
            failed_request_reason(
                "codex",
                "Provider prompt dispatch failed: You exceeded your current quota, please check your plan and billing details."
            ),
            "usage limit reached"
        );
        assert_eq!(
            failed_request_reason(
                "opencode",
                "Provider prompt dispatch failed: insufficient_quota"
            ),
            "usage limit reached"
        );
    }

    #[test]
    fn other_failures_keep_a_bounded_provider_diagnostic() {
        assert_eq!(
            failed_request_reason(
                "codex",
                "Provider prompt dispatch failed: stream disconnected\n before completion."
            ),
            "stream disconnected before completion"
        );
        assert_eq!(failed_request_reason("claude", "  "), "provider error");
        let long = format!("Provider prompt dispatch failed: {}", "x".repeat(400));
        assert_eq!(
            failed_request_reason("codex", &long).chars().count(),
            REASON_CHARS + 1
        );
    }

    #[test]
    fn note_quotes_a_bounded_excerpt_of_each_failed_request() {
        let one = failed("p1", "record r372-mcp-1,\n then r372-mcp-2");
        assert_eq!(
            failed_requests_note(std::slice::from_ref(&one)),
            "Your previous request (\"record r372-mcp-1, then r372-mcp-2\") failed (usage limit reached) and was not carried out. Do not act on it unless the user asks again; answer only the current request."
        );
        let long = failed("p2", &"word ".repeat(40));
        assert_eq!(long.excerpt.chars().count(), EXCERPT_CHARS + 1);
        assert!(long.excerpt.ends_with('…'));
        let both = failed_requests_note(&[one, long]);
        assert!(both.starts_with("Your previous requests (\"record r372-mcp-1, then r372-mcp-2\" (usage limit reached); \"word word"));
        assert!(both.contains("Do not act on them unless the user asks again"));
        assert_eq!(failed_requests_note(&[]), "");
    }

    #[test]
    fn notice_says_the_request_was_not_carried_out() {
        assert_eq!(
            failed_request_notice("usage limit reached"),
            "Request not carried out: usage limit reached. It was dropped; send it again to retry."
        );
    }
}

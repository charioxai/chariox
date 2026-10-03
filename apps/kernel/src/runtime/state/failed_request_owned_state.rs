//! A turn that failed before completing is marked "not carried out" in the
//! transcript and remembered, so the agent's next turn tells the provider,
//! which may resume a session still holding the request, to drop it.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn record_failed_request(
        &self,
        session_id: &str,
        provider_run_id: &str,
        adapter_key: &str,
        agent_id: &str,
        failed_prompt: &crate::session::PromptQueueItem,
        failure_message: &str,
    ) {
        let reason = crate::agent::failed_request_reason(adapter_key, failure_message);
        // The prompt-failure transcript entry, rendered by the web and the TUI.
        self.record_provider_failure_output(
            session_id,
            provider_run_id,
            agent_id,
            &crate::agent::failed_request_notice(&reason),
        );
        if let Err(error) = self.agent_store.record_failed_request_durably(
            &self.durable_state_store,
            agent_id,
            crate::agent::FailedRequest::new(failed_prompt.id(), failed_prompt.prompt(), reason),
        ) {
            crate::logging::warn_with_fields(
                "daemon.prompt_delivery",
                "failed to record the failed request note",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "prompt_id": failed_prompt.id(),
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) fn hidden_context_with_failed_requests(
        &self,
        agent_id: &str,
        hidden_system_context: &str,
    ) -> String {
        self.agent_store
            .hidden_context_with_failed_requests(agent_id, hidden_system_context)
    }

    /// Clears the one-time context the provider accepted with a turn: the
    /// provider-switch handoff and the failed-request note.
    pub(super) fn consume_delivered_turn_context(
        &self,
        session_id: &str,
        agent_id: &str,
        delivered_prompt_id: &str,
        target_run: &crate::provider::RuntimeProviderRun,
    ) {
        self.consume_pending_context_handoff(session_id, agent_id, target_run);
        self.consume_failed_requests(agent_id, delivered_prompt_id);
    }

    pub(super) fn consume_failed_requests(&self, agent_id: &str, delivered_prompt_id: &str) {
        if let Err(error) = self.agent_store.consume_failed_requests_durably(
            &self.durable_state_store,
            agent_id,
            delivered_prompt_id,
        ) {
            crate::logging::warn_with_fields(
                "daemon.prompt_delivery",
                "failed to clear the delivered failed request note",
                serde_json::json!({
                    "agent_id": agent_id,
                    "prompt_id": delivered_prompt_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::session::{DurablePromptPrivateState, PromptQueueItem, RuntimeSession};

pub(crate) const DURABLE_PROMPT_STATE_EVENT_KIND: &str = "session.prompt_state.updated";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DurablePromptStateEventPayload {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) active_prompt: Option<PromptQueueItem>,
    pub(crate) queued_prompts: VecDeque<PromptQueueItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) private_states: Vec<DurablePromptPrivateState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_prompt_sent_at_ms: Option<u64>,
}

impl DurablePromptStateEventPayload {
    pub(crate) fn capture(session: &RuntimeSession, agent_id: &str) -> Self {
        let active_prompt = session.active_prompt_for_agent(agent_id).cloned();
        let queued_prompts = session
            .queued_prompts_for_agent(agent_id)
            .cloned()
            .unwrap_or_default();
        let private_states = active_prompt
            .iter()
            .chain(queued_prompts.iter())
            .filter_map(|prompt| DurablePromptPrivateState::from_prompt(session.id(), prompt))
            .collect();
        Self {
            session_id: session.id().to_string(),
            agent_id: agent_id.to_string(),
            active_prompt,
            queued_prompts,
            private_states,
            last_prompt_sent_at_ms: session
                .agents()
                .iter()
                .find(|agent| agent.id() == agent_id)
                .and_then(|agent| agent.last_prompt_sent_at_ms()),
        }
    }

    pub(crate) fn restore_private_states(&mut self) {
        for prompt in self
            .active_prompt
            .iter_mut()
            .chain(self.queued_prompts.iter_mut())
        {
            let Some(private) = self
                .private_states
                .iter()
                .find(|private| private.prompt_id == prompt.id())
            else {
                continue;
            };
            prompt.restore_durable_private_state(private);
        }
    }
}

pub(crate) fn append_durable_prompt_state_event(
    store: &DurableKernelStateStore,
    session: &RuntimeSession,
    agent_id: &str,
) -> Result<(), DaemonError> {
    let payload = serde_json::to_value(DurablePromptStateEventPayload::capture(session, agent_id))
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.encode_prompt_state_event",
            message: error.to_string(),
        })?;
    store.append_event(
        DURABLE_PROMPT_STATE_EVENT_KIND,
        Some(session.id().to_string()),
        payload,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{DurablePromptDeliveryPhase, PromptStatus};

    #[test]
    fn durable_prompt_event_round_trips_private_delivery_state() {
        let mut session = RuntimeSession::new(
            "session-1",
            None,
            "workspace-1",
            "worktree-1",
            "machine-1",
            "daemon-1",
        );
        let mut prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "visible prompt",
            PromptStatus::Running,
        )
        .with_hidden_system_context("private context")
        .with_durable_operation("command-1", "fingerprint-1");
        prompt.set_durable_delivery(
            DurablePromptDeliveryPhase::Delivered,
            Some("provider-run-1".to_string()),
            Some("provider-session-1".to_string()),
        );
        let recovery_operation_id = prompt.begin_durable_recovery_operation();
        assert!(prompt.mark_durable_recovery_phase(
            &recovery_operation_id,
            DurablePromptDeliveryPhase::Dispatching,
        ));
        session.mirror_agent_prompt_state(
            "agent-1",
            Some(prompt),
            std::collections::VecDeque::new(),
        );

        let encoded =
            serde_json::to_value(DurablePromptStateEventPayload::capture(&session, "agent-1"))
                .expect("event should encode");
        assert_eq!(encoded["private_states"][0]["delivery_phase"], "delivered");
        assert_eq!(
            encoded["private_states"][0]["recovery_operation_id"],
            recovery_operation_id
        );
        assert!(encoded["active_prompt"].get("private_metadata").is_none());
        let mut restored: DurablePromptStateEventPayload =
            serde_json::from_value(encoded).expect("event should decode");
        restored.restore_private_states();
        let prompt = restored
            .active_prompt
            .expect("active prompt should restore");

        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            prompt.durable_delivery_provider_run_id(),
            Some("provider-run-1")
        );
        assert_eq!(
            prompt.durable_delivery_provider_session_id(),
            Some("provider-session-1")
        );
        assert_eq!(prompt.hidden_system_context(), "private context");
        assert_eq!(
            prompt.durable_recovery_operation_id(),
            Some(recovery_operation_id.as_str())
        );
        assert_eq!(
            prompt.durable_recovery_phase(),
            Some(DurablePromptDeliveryPhase::Dispatching)
        );
    }
}

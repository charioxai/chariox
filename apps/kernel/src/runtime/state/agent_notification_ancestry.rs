//! Notification lineage follows the active prompt when an agent invokes a workflow.
use super::*;
use crate::session::WorkflowPublicationInvocationEnvelope;

impl KernelRuntimeOwnedState {
    pub(super) fn inherit_agent_notification_ancestry(
        &self,
        creator: Option<&str>,
        endpoint_ref: &str,
        invocation: Option<WorkflowPublicationInvocationEnvelope>,
    ) -> Result<Option<WorkflowPublicationInvocationEnvelope>, DaemonError> {
        let Some(agent) = creator else {
            return Ok(invocation);
        };
        let actor = self.agent_store.get_agent(agent)?;
        let session = self.session_store.get_session(actor.session_id())?;
        let Some(prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
        else {
            return Ok(invocation);
        };
        let Some(mut ancestry) = self
            .durable_state_store
            .agent_notification_ancestry(session.id(), prompt.id())?
        else {
            return Ok(invocation);
        };
        let identity = format!("agent:{agent}");
        if ancestry.len() >= 256 {
            return Err(crate::durable_state::workflow_notifications::error(
                "workflow_notification_ancestry_limit",
            ));
        }
        if !ancestry.contains(&identity) {
            ancestry.push(identity);
        }
        let mut invocation = invocation.unwrap_or_else(|| WorkflowPublicationInvocationEnvelope {
            publication_id: String::new(),
            hook_id: None,
            invocation_id: prompt.id().into(),
            transport: "agent_notification".into(),
            endpoint_id: endpoint_ref.into(),
            queue_ref: None,
            input: serde_json::json!({}),
            artifacts: Vec::new(),
            mode: None,
            caller: serde_json::json!({"kind":"agent","agent_id":agent}),
        });
        // Kernel-owned lineage cannot be replaced by invocation input supplied by the caller.
        if !invocation.input.is_object() {
            invocation.input = serde_json::json!({"input":invocation.input});
        }
        invocation.input["ancestry"] = serde_json::json!(ancestry);
        Ok(Some(invocation))
    }
}

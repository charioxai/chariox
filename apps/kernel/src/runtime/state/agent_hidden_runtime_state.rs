//! MP-08 / MP-09 / MP-10 / MP-11: keep an agent out of the Room's panes while its work goes on.
use super::*;

impl KernelRuntimeState {
    /// Hiding changes only what clients show: the run, the queue, wakes and triggers are untouched.
    pub(crate) fn hide_agent_in_session(
        &self,
        request: &crate::local::HideAgentRequest,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.authorize_current_external_command()?;
        let agent =
            self.owned
                .ensure_agent_owner(&request.agent_id, caller_user_id, "hide agent")?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
            });
        }
        if agent.archived() || agent.hidden() == request.hidden {
            return Ok(agent);
        }
        let agent = self
            .owned
            .agent_store
            .set_agent_hidden(agent.id(), request.hidden)?;
        self.record_agent_durable_event("agent.updated", &agent, None)?;
        Ok(agent)
    }
}

impl KernelRuntimeOwnedState {
    /// A hidden agent comes back in every client's Room: when its last active or queued prompt is over, or when it needs a decision.
    pub(super) fn unhide_agent(&self, agent_id: &str) -> Result<(), DaemonError> {
        let Ok(agent) = self.agent_store.get_agent(agent_id) else {
            return Ok(());
        };
        if !agent.hidden() {
            return Ok(());
        }
        let agent = self.agent_store.set_agent_hidden(agent_id, false)?;
        self.durable_state_store.append_event(
            "agent.updated",
            Some(agent.id().to_string()),
            serde_json::json!({ "agent": agent, "capability_name": serde_json::Value::Null }),
        )?;
        Ok(())
    }

    /// A workflow settlement removes the prompt without the mirror above, so it asks whether any work is left.
    pub(super) fn unhide_agent_without_prompt_work(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        if session.active_prompt_for_agent(agent_id).is_some()
            || session
                .queued_prompts_for_agent(agent_id)
                .is_some_and(|held| !held.is_empty())
        {
            return Ok(());
        }
        self.unhide_agent(agent_id)
    }
}

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn archive_agent_in_session(
        &self,
        request: &crate::local::ArchiveAgentRequest,
        caller_user_id: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        self.authorize_current_external_command()?;
        let agent =
            self.owned
                .ensure_agent_owner(&request.agent_id, caller_user_id, "archive agent")?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.agent_id.clone(),
            });
        }
        let session = self.owned.session_store.get_session(&request.session_id)?;
        let agent = self.owned.prompt_state_owner.change_agent_archive(
            &session,
            agent.id(),
            request.archived,
            || {
                let current = self.owned.agent_store.get_agent(agent.id())?;
                if request.archived && current.is_processing() {
                    return Err(DaemonError::LocalTransport {
                        operation: "archive agent",
                        message: "Stop this agent before archiving it.".into(),
                    });
                }
                let previous = self
                    .owned
                    .agent_store
                    .get_session_agents(&request.session_id);
                let focused = self
                    .owned
                    .session_store
                    .get_session(&request.session_id)?
                    .focused_agent_id()
                    .map(str::to_string);
                let agent = self.owned.agent_store.archive_agent(
                    agent.id(),
                    request.archived,
                    &mut self.owned.session_store.write(),
                )?;
                if let Err(error) = self.record_agent_durable_event("agent.updated", &agent, None) {
                    for agent in previous {
                        self.owned.agent_store.restore_agent(agent);
                    }
                    self.owned
                        .session_store
                        .write()
                        .set_focused_agent(&request.session_id, focused)?;
                    return Err(error);
                }
                Ok(agent)
            },
        )?;
        self.owned
            .sync_focused_provider_run_if_idle(&request.session_id)?;
        if !request.archived {
            self.schedule_agent_lifecycle_sweep();
        }
        Ok(agent)
    }
}

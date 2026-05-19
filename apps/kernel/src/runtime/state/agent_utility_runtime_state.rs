use super::*;

impl KernelRuntimeState {
    pub(crate) async fn agent_utility_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<
        (
            crate::agent::AgentInstance,
            crate::provider::RuntimeProviderRun,
        ),
        DaemonError,
    > {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self
            .owned
            .agent_store
            .get_session_agents(session_id)
            .iter()
            .find(|agent| agent.id() == agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!("agent `{agent_id}` does not belong to session `{session_id}`"),
            })?;
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!(
                    "agent `{agent_id}` is remote-backed; hidden utilities must run on its worker kernel"
                ),
            });
        }
        if session.active_prompt_for_agent(agent_id).is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("agent `{agent_id}` is busy"),
            });
        }
        let provider_run = if let Some(provider_run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        {
            provider_run
        } else {
            let session_id = session_id.to_string();
            let agent_id = agent_id.to_string();
            let provider_run_id = self
                .with_app_side_effect(move |app| {
                    app.ensure_prompt_provider_run_for_agent(&session_id, &agent_id)
                })
                .await?;
            self.owned.provider_store.get_run(&provider_run_id)?
        };
        Ok((agent, provider_run))
    }
}

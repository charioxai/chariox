use crate::error::DaemonError;

// MP-08/MP-10/MP-11: lease replicas belong to the authenticated lease
// owner. Ordinary home clients keep the existing Cloud-to-local alias.
impl crate::app::DaemonApp {
    pub(crate) fn provider_account_owner_for_execution(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Result<String, DaemonError> {
        let session = self.sessions.get_session(session_id)?;
        let agent = agent_id.map(|id| self.agents.get_agent(id)).transpose()?;
        if let Some(agent) = agent.as_ref() {
            if agent.session_id() != session_id {
                return Err(DaemonError::AgentNotInSession {
                    session_id: session_id.into(),
                    agent_id: agent.id().into(),
                });
            }
        }
        let owner = agent
            .as_ref()
            .map(|agent| agent.owner_user_id())
            .unwrap_or_else(|| session.owner_user_id());
        let mut bindings = self.leased_agents.values().filter(|leased| {
            leased.backing_session_id == session_id
                && agent_id == Some(leased.backing_agent_id.as_str())
        });
        if let Some(leased) = bindings.next() {
            let lease = self.execution_leases.get(&leased.lease_id).ok_or_else(|| {
                DaemonError::ExecutionLeaseNotFound {
                    lease_id: leased.lease_id.clone(),
                }
            })?;
            if bindings.next().is_some()
                || lease.owner_user_id != owner
                || lease.owner_user_id != session.owner_user_id()
            {
                return Err(DaemonError::LocalTransport {
                    operation: "resolve leased provider account authority",
                    message: "worker backing identity does not match one execution lease owner"
                        .into(),
                });
            }
            return Ok(lease.owner_user_id.clone());
        }
        Ok(crate::account_profile::provider_account_authority_owner_user_id(&self.config, owner))
    }
}

//! MP-01/MP-08/MP-11: app account checks derive lease provenance from stored bindings.
use super::*;

impl DaemonApp {
    pub(crate) fn is_leased_backing_agent(&self, session_id: &str, agent_id: &str) -> bool {
        self.leased_agents.values().any(|lease| {
            lease.backing_agent_id == agent_id && lease.backing_session_id == session_id
        })
    }

    pub(crate) fn require_agent_provider_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let lease_owned = self.is_leased_backing_agent(agent.session_id(), agent.id());
        let owner = crate::account_profile::provider_account_execution_owner_user_id(
            &self.config,
            agent.owner_user_id(),
            lease_owned,
        );
        let run = lease_owned
            .then(|| {
                self.providers
                    .get_run_for_agent(agent.session_id(), agent.id())
            })
            .flatten();
        crate::account_profile::require_agent_account_authenticated(
            &self.provider_account_profiles,
            &owner,
            agent,
            run.as_ref(),
            operation,
        )
    }
}

//! MP-01/MP-08/MP-11: execution bindings select the shared account owner policy.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn require_agent_provider_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        self.require_agent_account_authenticated(agent, None, operation)
    }

    pub(super) fn provider_account_owner_for_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<String, DaemonError> {
        if self
            .provider_run_projection
            .is_leased_provider_run(run.id())
        {
            self.provider_account_profiles.get(
                run.owner_user_id(),
                run.provider(),
                run.account_profile(),
            )?;
            return Ok(run.owner_user_id().to_string());
        }
        crate::account_profile::provider_account_authority_for_run(
            &self.config_projection.snapshot(),
            &self.provider_account_profiles,
            run,
        )
    }

    pub(super) fn provider_account_owner_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        if let Some(run) = self
            .provider_run_projection
            .get_for_agent(agent.session_id(), agent.id())
        {
            return self.provider_account_owner_for_run(&run);
        }
        crate::account_profile::provider_account_authority_owner_for_profile(
            &self.config_projection.snapshot(),
            &self.provider_account_profiles,
            agent.owner_user_id(),
            agent.provider(),
            agent.provider_account_profile(),
        )
    }
}

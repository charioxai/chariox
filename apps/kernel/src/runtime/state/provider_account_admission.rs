//! MP-08 / MP-10 / MP-11: prompt admission follows the selected run's account.
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn require_agent_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        selected_run: Option<&crate::provider::RuntimeProviderRun>,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let config = self.config_projection.snapshot();
        let Some(provider) = crate::provider::canonical_provider_family(agent.provider()) else {
            return Ok(());
        };
        let current = selected_run.cloned().or_else(|| {
            self.provider_store
                .get_run_for_agent(agent.session_id(), agent.id())
        });
        if let Some(run) = current.filter(|run| {
            agent.remote_execution().is_none()
                && run.agent_instance_id() == Some(agent.id())
                && run.session_id() == agent.session_id()
                && run.owner_user_id() == agent.owner_user_id()
                && run.provider() == agent.provider()
                && run.state() != crate::provider::ProviderRunState::Ended
                && agent.model().is_none_or(|model| model == run.model())
                && (agent.provider_account_profile() == run.account_profile()
                    || agent.provider_account_profile()
                        == crate::account_profile::provider_account_selection_for_run(run)
                    || (agent.provider_account_profile() == "default"
                        && run.resolved_provider_account.is_none()))
        }) {
            let owner = crate::account_profile::provider_account_authority_for_run(
                &config,
                &self.provider_account_profiles,
                &run,
            )?;
            // Restored runs recover their namespace from provider-local roots.
            // A home alias must never adopt a leased replica's namespace.
            let selection_matches = agent.provider_account_profile() == run.account_profile()
                || agent.provider_account_profile()
                    == crate::account_profile::provider_account_selection_for_run(&run);
            let alias_owner = crate::account_profile::provider_account_authority_owner_for_profile(
                &config,
                &self.provider_account_profiles,
                agent.owner_user_id(),
                provider,
                agent.provider_account_profile(),
            )?;
            if selection_matches || owner == alias_owner {
                self.provider_account_profiles.require_authenticated(
                    &owner,
                    provider,
                    run.account_profile(),
                    agent.model(),
                    operation,
                )?;
                return Ok(());
            }
        }
        self.provider_account_profiles
            .require_agent_authenticated(&config, agent, operation)
    }
}

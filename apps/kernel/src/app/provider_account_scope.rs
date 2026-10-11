//! MP-08 / MP-10 / MP-11: prompt admission retains the kernel's lease account namespace.
use super::*;
use crate::account_profile::ProviderAccountLaunchScope;

impl DaemonApp {
    pub(super) fn provider_account_scope_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> ProviderAccountLaunchScope {
        let run = self
            .providers
            .get_run_for_agent(agent.session_id(), agent.id());
        let leased = run.as_ref().is_some_and(|run| {
            self.provider_run_projection
                .is_leased_provider_run(run.id())
        });
        ProviderAccountLaunchScope::for_agent_run(agent, run.as_ref(), leased)
    }

    pub(super) fn require_agent_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        self.provider_account_profiles
            .require_agent_authenticated_with_scope(
                &self.config,
                agent,
                operation,
                self.provider_account_scope_for_agent(agent),
            )
    }
}

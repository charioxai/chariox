//! MP-08 / MP-10 / MP-11: prompt admission retains the kernel's lease account namespace.
use super::*;
use crate::account_profile::ProviderAccountLaunchScope;

impl DaemonApp {
    pub(super) fn provider_account_scope_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> ProviderAccountLaunchScope {
        let (run, leased) = self.agent_run_lease_marker(agent);
        ProviderAccountLaunchScope::for_agent_run(agent, run.as_ref(), leased)
    }

    fn agent_run_lease_marker(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> (Option<crate::provider::RuntimeProviderRun>, bool) {
        let run = self
            .providers
            .get_run_for_agent(agent.session_id(), agent.id());
        let leased = run.as_ref().is_some_and(|run| {
            self.provider_run_projection
                .is_leased_provider_run(run.id())
        });
        (run, leased)
    }

    pub(super) fn require_agent_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let (run, leased) = self.agent_run_lease_marker(agent);
        if ProviderAccountLaunchScope::lease_run_holds_home_claude_credential(
            agent,
            run.as_ref(),
            leased,
        ) {
            return Ok(());
        }
        self.provider_account_profiles
            .require_agent_authenticated_with_scope(
                &self.config,
                agent,
                operation,
                ProviderAccountLaunchScope::for_agent_run(agent, run.as_ref(), leased),
            )
    }
}

//! MP-08 / MP-10 / MP-11: preserve the admitted lease account through local queueing.
use super::*;
pub(super) use crate::account_profile::ProviderAccountLaunchScope;

impl KernelRuntimeOwnedState {
    pub(super) fn require_agent_account_authenticated(
        &self,
        agent: &crate::agent::AgentInstance,
        operation: &'static str,
    ) -> Result<(), DaemonError> {
        let run = self
            .provider_store
            .get_run_for_agent(agent.session_id(), agent.id());
        let leased = run.as_ref().is_some_and(|run| {
            self.provider_run_projection
                .is_leased_provider_run(run.id())
        });
        if ProviderAccountLaunchScope::lease_run_holds_home_claude_credential(
            agent,
            run.as_ref(),
            leased,
        ) {
            return Ok(());
        }
        self.provider_account_profiles
            .require_agent_authenticated_with_scope(
                &self.config_projection.snapshot(),
                agent,
                operation,
                ProviderAccountLaunchScope::for_agent_run(agent, run.as_ref(), leased),
            )
    }
}

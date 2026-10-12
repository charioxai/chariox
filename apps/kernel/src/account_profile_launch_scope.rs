//! MP-08/MP-10/MP-11: keep lease replicas separate from host-owner accounts.
#[derive(Clone, Copy)]
pub(crate) enum ProviderAccountLaunchScope {
    Home,
    // Only the authenticated leased-provider launch entry uses this scope.
    // A Cloud login on the worker must not alias the home lease owner's replica
    // into the worker's local account namespace.
    LeaseReplica,
}

impl ProviderAccountLaunchScope {
    // The marker is kernel-owned and set only after an authenticated lease launch.
    // A projected remote run or an ordinary run cannot select a worker replica.
    pub(crate) fn for_agent_run(
        agent: &crate::agent::AgentInstance,
        run: Option<&crate::provider::RuntimeProviderRun>,
        marked_leased: bool,
    ) -> Self {
        if marked_leased
            && agent.remote_execution().is_none()
            && run.is_some_and(|run| {
                run.session_id() == agent.session_id()
                    && run.agent_instance_id() == Some(agent.id())
                    && run.owner_user_id() == agent.owner_user_id()
            })
        {
            Self::LeaseReplica
        } else {
            Self::Home
        }
    }

    // A leased Claude replica runs on the home's per-launch credential, so its
    // worker profile is never logged in. Only its live marked run proves that
    // credential; without one, launching still requires the home credential.
    pub(crate) fn lease_run_holds_home_claude_credential(
        agent: &crate::agent::AgentInstance,
        run: Option<&crate::provider::RuntimeProviderRun>,
        marked_leased: bool,
    ) -> bool {
        crate::provider::canonical_provider_family(agent.provider()) == Some("claude")
            && matches!(
                Self::for_agent_run(agent, run, marked_leased),
                Self::LeaseReplica
            )
            && run.is_some_and(|run| {
                matches!(
                    run.state(),
                    crate::provider::ProviderRunState::Starting
                        | crate::provider::ProviderRunState::Running
                )
            })
    }

    pub(crate) fn owner_user_id(self, config: &crate::config::DaemonConfig, owner: &str) -> String {
        match self {
            Self::Home => {
                crate::account_profile::provider_account_authority_owner_user_id(config, owner)
            }
            Self::LeaseReplica => owner.to_string(),
        }
    }
}

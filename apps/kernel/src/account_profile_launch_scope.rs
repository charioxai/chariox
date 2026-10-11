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

    pub(crate) fn owner_user_id(self, config: &crate::config::DaemonConfig, owner: &str) -> String {
        match self {
            Self::Home => {
                crate::account_profile::provider_account_authority_owner_user_id(config, owner)
            }
            Self::LeaseReplica => owner.to_string(),
        }
    }
}

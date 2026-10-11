//! MP-08/MP-10/MP-11: keep lease replicas separate from host-owner accounts.
#[derive(Clone, Copy)]
pub(super) enum ProviderAccountLaunchScope {
    Home,
    // Only the authenticated leased-provider launch entry uses this scope.
    // A Cloud login on the worker must not alias the home lease owner's replica
    // into the worker's local account namespace.
    LeaseReplica,
}

impl ProviderAccountLaunchScope {
    pub(super) fn owner_user_id(self, config: &crate::config::DaemonConfig, owner: &str) -> String {
        match self {
            Self::Home => {
                crate::account_profile::provider_account_authority_owner_user_id(config, owner)
            }
            Self::LeaseReplica => owner.to_string(),
        }
    }
}

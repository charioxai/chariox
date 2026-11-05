//! Account usage reads through the official Codex app-server. Claude usage
//! remains an observation from running turns, never a direct HTTP request.
use std::sync::Arc;

use crate::account_profile::{ProviderAccountProfileRegistry, ProviderAccountUsageSnapshot};
use crate::provider::{
    ensure_codex_account_endpoint, CodexClient, ProviderRunState, ProviderUsageReadError,
};

use super::{ProviderUsageReader, ProviderUsageRefresher, UsageAccountKey};

struct KernelUsageSources {
    registry: ProviderAccountProfileRegistry,
}

impl ProviderUsageReader for KernelUsageSources {
    fn read(
        &self,
        account: &UsageAccountKey,
    ) -> Result<ProviderAccountUsageSnapshot, ProviderUsageReadError> {
        let environment = self.registry.resolve_environment(
            &account.owner_user_id,
            &account.provider,
            &account.profile_id,
        )?;
        match crate::provider::canonical_provider_family(&account.provider) {
            Some("codex") => {
                let endpoint = ensure_codex_account_endpoint(
                    &account.owner_user_id,
                    &account.profile_id,
                    environment,
                )?;
                let client = CodexClient::new("provider-usage-refresh", endpoint)?;
                Ok(client.usage_snapshot(&account.profile_id)?)
            }
            _ => Err(ProviderUsageReadError::new(
                "this provider has no account usage source",
            )),
        }
    }
}

impl ProviderUsageRefresher {
    pub(crate) fn for_kernel(
        registry: ProviderAccountProfileRegistry,
        provider_store: crate::provider::ProviderProcessServiceStore,
    ) -> Arc<Self> {
        let reader = KernelUsageSources {
            registry: registry.clone(),
        };
        Self::new(
            registry,
            Box::new(reader),
            Box::new(move |account| {
                provider_store.list_runs().iter().any(|run| {
                    matches!(
                        run.state(),
                        ProviderRunState::Starting | ProviderRunState::Running
                    ) && run.account_profile() == account.profile_id
                        && crate::provider::canonical_provider_family(run.provider())
                            == Some(account.provider.as_str())
                })
            }),
        )
    }
}

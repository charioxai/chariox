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
        runs: crate::runtime::projection::ProviderRunProjectionStore,
    ) -> Arc<Self> {
        let reader = KernelUsageSources {
            registry: registry.clone(),
        };
        Self::new(
            registry,
            Box::new(reader),
            Box::new(move || {
                // Scheduling uses the published projection, never the provider
                // authority mutex held during launch and other operations.
                runs.list()
                    .into_iter()
                    .filter(|run| {
                        matches!(
                            run.state(),
                            ProviderRunState::Starting | ProviderRunState::Running
                        )
                    })
                    .filter_map(|run| {
                        Some((
                            crate::provider::canonical_provider_family(run.provider())?.to_string(),
                            run.account_profile().to_string(),
                        ))
                    })
                    .collect()
            }),
        )
    }
}

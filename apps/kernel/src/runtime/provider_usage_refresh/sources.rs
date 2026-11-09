//! Account usage reads through official provider tools: the Codex app-server,
//! and for Claude one minimal turn of the Claude CLI. Never a direct HTTP request.
use std::sync::Arc;

use crate::account_profile::{ProviderAccountProfileRegistry, ProviderAccountUsageSnapshot};
use crate::provider::{
    ensure_codex_account_endpoint, CodexClient, ProviderRunState, ProviderUsageReadError,
};
use crate::runtime::projection::DaemonConfigProjectionStore;

use super::{ProviderUsageReader, ProviderUsageRefresher, UsageAccountKey};

struct KernelUsageSources {
    registry: ProviderAccountProfileRegistry,
    config: DaemonConfigProjectionStore,
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
            Some("claude") => {
                let mut environment = environment;
                for (name, value) in crate::provider::resolve_provider_account_credentials(
                    &self.config.snapshot(),
                    &account.owner_user_id,
                    &account.provider,
                    &account.profile_id,
                )?
                .iter()
                {
                    environment.insert(name.to_string(), value.to_string());
                }
                let executable = crate::provider::resolve_claude_executable()?;
                let read = crate::provider::read_claude_rate_limits(&executable, &environment);
                if let Some(token) = environment.get_mut(crate::provider::CLAUDE_OAUTH_TOKEN_ENV) {
                    zeroize::Zeroize::zeroize(token);
                }
                Ok(read?)
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
        config: DaemonConfigProjectionStore,
    ) -> Arc<Self> {
        let reader = KernelUsageSources {
            registry: registry.clone(),
            config,
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

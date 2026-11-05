//! The kernel's real account usage sources: the official Codex app-server for
//! Codex, and for Claude the OAuth usage read of an enrolled token or the
//! official CLI's usage request for a login the CLI manages itself.
use std::sync::Arc;

use crate::account_profile::{ProviderAccountProfileRegistry, ProviderAccountUsageSnapshot};
use crate::provider::{
    ensure_codex_account_endpoint, probe_claude_account_usage, read_claude_oauth_usage,
    resolve_claude_executable, resolve_provider_account_credentials, CodexClient, ProviderRunState,
    ProviderUsageReadError, CLAUDE_OAUTH_TOKEN_ENV,
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
                let credentials = resolve_provider_account_credentials(
                    &self.config.snapshot(),
                    &account.owner_user_id,
                    &account.provider,
                    &account.profile_id,
                )?;
                let token = credentials
                    .iter()
                    .find(|(name, _)| *name == CLAUDE_OAUTH_TOKEN_ENV)
                    .map(|(_, token)| token);
                match token {
                    Some(token) => read_claude_oauth_usage(token, &account.profile_id),
                    None => probe_claude_account_usage(
                        &resolve_claude_executable()?,
                        &account.profile_id,
                        &environment,
                    ),
                }
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
        config: DaemonConfigProjectionStore,
        provider_store: crate::provider::ProviderProcessServiceStore,
    ) -> Arc<Self> {
        let reader = KernelUsageSources {
            registry: registry.clone(),
            config,
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

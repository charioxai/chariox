//! Account usage reads through official provider tools: the Codex app-server,
//! and for Claude one minimal turn of the Claude CLI. Never a direct HTTP request.
use std::sync::Arc;

use crate::account_profile::{ProviderAccountProfileRegistry, ProviderAccountUsageSnapshot};
use crate::provider::{
    ensure_codex_account_endpoint, CodexClient, ProviderRunState, ProviderUsageReadError,
    RuntimeProviderRun,
};
use crate::runtime::projection::DaemonConfigProjectionStore;
use crate::session::RuntimeSession;

use super::{AccountSet, ProviderUsageReader, ProviderUsageRefresher, UsageAccountKey};

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
        sessions: crate::runtime::projection::SessionStateProjectionStore,
        config: DaemonConfigProjectionStore,
    ) -> Arc<Self> {
        let reader = KernelUsageSources {
            registry: registry.clone(),
            config,
        };
        let running_runs = runs.clone();
        // Scheduling uses the published projections, never the provider
        // authority mutex held during launch and other operations.
        let running = move || {
            let mut accounts = AccountSet::new();
            for run in running_runs.list() {
                if matches!(
                    run.state(),
                    ProviderRunState::Starting | ProviderRunState::Running
                ) {
                    accounts.extend(account_of(&run));
                }
            }
            accounts
        };
        // A focused run stays Running between prompts, so only prompt work the
        // kernel owns shows that its agent is doing something.
        let prompting = move || {
            let mut accounts = AccountSet::new();
            for run in runs.list() {
                let busy = matches!(
                    run.state(),
                    ProviderRunState::Starting | ProviderRunState::Running
                ) && run.agent_instance_id().is_some_and(|agent_id| {
                    sessions
                        .get_shared(run.session_id())
                        .is_some_and(|session| agent_has_prompt_work(&session, agent_id))
                });
                if busy {
                    accounts.extend(account_of(&run));
                }
            }
            accounts
        };
        Self::new(
            registry,
            Box::new(reader),
            Box::new(running),
            Box::new(prompting),
        )
    }
}

fn account_of(run: &RuntimeProviderRun) -> Option<(String, String)> {
    Some((
        crate::provider::canonical_provider_family(run.provider())?.to_string(),
        run.account_profile().to_string(),
    ))
}

fn agent_has_prompt_work(session: &RuntimeSession, agent_id: &str) -> bool {
    session.active_prompt_for_agent(agent_id).is_some()
        || session
            .queued_prompts_for_agent(agent_id)
            .is_some_and(|queued| !queued.is_empty())
}

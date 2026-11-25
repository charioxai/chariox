//! Exact selected-account transfer before an existing worker profile changes.

use super::*;
use crate::account_profile::{
    ProviderAccountMaterializationState, ProviderAccountMaterializationStatus,
    ProviderAccountMaterializationTargetKind,
};
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

impl KernelRuntimeState {
    /// Returns the worker's profile id for the account: the home's when the worker made its copy, its own when it already held a login from that account.
    pub(super) async fn ensure_remote_profile_account(
        &self,
        agent_id: &str,
        update: &owned::OwnedRemoteAgentProfileUpdate,
        config: &crate::config::DaemonConfig,
    ) -> Result<String, DaemonError> {
        let Some(provider) = crate::provider::canonical_provider_family(&update.provider) else {
            return Ok(update.account_profile.clone());
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let registry = &self.owned.provider_account_profiles;
        let target_kind = if self
            .owned
            .slice_store
            .resolve_by_worker_kernel_ref(&update.worker_kernel_id)
            .is_some()
        {
            ProviderAccountMaterializationTargetKind::Slice
        } else {
            ProviderAccountMaterializationTargetKind::Worker
        };
        let result = async {
            let mut materialization =
                registry.export_materialization(&owner, provider, &update.account_profile)?;
            // A resolved stable ID must cross both account and profile requests.
            if materialization.profile.profile_id != update.account_profile {
                return Err(DaemonError::LocalTransport {
                    operation: "materialize remote profile account",
                    message: "selected account identity changed; select the account again".into(),
                });
            }
            // Cloud-owner aliases are local registry details, not lease owners.
            materialization.profile.owner_user_id = agent.owner_user_id().to_string();
            let credentialless = materialization.is_credentialless_opencode();
            let response = self
                .send_remote_profile_request(
                    config,
                    &update.worker_kernel_id,
                    RelayPeerRequest::EnsureRemoteProviderAccount {
                        context: RemoteProviderAccountSyncContext {
                            home_kernel_id: config.daemon_id.clone(),
                            home_session_id: agent.session_id().to_string(),
                            home_agent_id: agent_id.to_string(),
                            execution_lease_id: update.execution_lease_id.clone(),
                        },
                        materialization,
                    },
                )
                .await?;
            match response {
                RelayPeerResponse::RemoteProviderAccountEnsured {
                    provider: confirmed_provider,
                    account_profile,
                } if confirmed_provider == provider
                    && !account_profile.trim().is_empty()
                    && account_profile != "default" =>
                {
                    Ok((account_profile, credentialless))
                }
                _ => Err(DaemonError::LocalTransport {
                    operation: "materialize remote profile account",
                    message:
                        "worker did not confirm the selected provider account; profile unchanged"
                            .into(),
                }),
            }
        }
        .await;
        // The home's record says "installed" only for the copy it made; a login the worker already held is asked about again.
        let status = if result.as_ref().is_ok_and(|(confirmed, credentialless)| {
            *credentialless || confirmed != &update.account_profile
        }) {
            Ok(())
        } else {
            registry
                .update_materialization_status(
                    &owner,
                    provider,
                    &update.account_profile,
                    ProviderAccountMaterializationStatus {
                        target_kind,
                        target_ref: update.worker_kernel_id.clone(),
                        state: if result.is_ok() {
                            ProviderAccountMaterializationState::Materialized
                        } else {
                            ProviderAccountMaterializationState::Error
                        },
                        observed_at_ms: crate::session::unix_epoch_ms(),
                        last_error: result.as_ref().err().map(|_| {
                            "selected account transfer failed; profile unchanged".to_string()
                        }),
                    },
                )
                .map(|_| ())
        };
        // Never destroy an existing lease or discard queued work on failure.
        let (confirmed, _) = result?;
        status?;
        Ok(confirmed)
    }

    /// The worker answered that it no longer holds the account the home recorded
    /// as installed there. Hand it over again under the same lease, so the same
    /// prompt can be submitted once more.
    pub(super) async fn reinstall_remote_prompt_account(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(&dispatch.agent_id)?;
        let binding =
            agent
                .remote_execution()
                .cloned()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "reinstall remote provider account",
                    message: "the agent no longer runs on a worker".into(),
                })?;
        let config = self.config_snapshot().await;
        let update = owned::OwnedRemoteAgentProfileUpdate {
            worker_kernel_id: binding.worker_kernel_id.clone(),
            execution_lease_id: binding.execution_lease_id.clone(),
            leased_agent_id: binding.leased_agent_id.clone(),
            relay_url: binding.relay_url.clone(),
            relay_token: binding.relay_token.clone(),
            provider: agent.provider().to_string(),
            account_profile: agent.provider_account_profile().to_string(),
            model: None,
            effort: None,
        };
        let mut config = config;
        if let (Some(url), Some(token)) = (update.relay_url.clone(), update.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        let installed = self
            .ensure_remote_profile_account(&dispatch.agent_id, &update, &config)
            .await?;
        if installed != update.account_profile {
            // The worker keeps its own login for this account; the leased agent
            // still names the profile it was placed with.
            return Err(DaemonError::LocalTransport {
                operation: "reinstall remote provider account",
                message: format!(
                    "the worker holds this account as `{installed}`, not `{}`; place the agent again",
                    update.account_profile
                ),
            });
        }
        Ok(())
    }
}

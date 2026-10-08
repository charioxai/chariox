//! Exact selected-account transfer before an existing worker profile changes.

use super::*;
use crate::account_profile::{
    ProviderAccountMaterializationState, ProviderAccountMaterializationStatus,
    ProviderAccountMaterializationTargetKind,
};
use crate::transport::relay_peer::RemoteProviderAccountSyncContext;

impl KernelRuntimeState {
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
        let profile = registry.get(&owner, provider, &update.account_profile)?;
        if let Some(receiving_account) =
            profile.installed_account_id_at(target_kind, &update.worker_kernel_id)
        {
            return Ok(receiving_account.to_string());
        }
        let result = async {
            let mut materialization =
                registry.export_materialization(&owner, provider, &update.account_profile)?;
            if materialization.profile.profile_id != update.account_profile {
                return Err(DaemonError::LocalTransport {
                    operation: "materialize remote profile account",
                    message: "selected account identity changed; select the account again".into(),
                });
            }
            // Cloud-owner aliases are local registry details, not lease owners.
            materialization.profile.owner_user_id = agent.owner_user_id().to_string();
            let binding = agent
                .remote_execution()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "issue account copy",
                    message: "worker binding disappeared".into(),
                })?;
            let expected_copy = registry.prepare_account_copy(
                &owner,
                &materialization,
                target_kind,
                &binding.worker_machine_id,
                &update.worker_kernel_id,
            )?;
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
            let RelayPeerResponse::RemoteProviderAccountEnsured {
                copy,
                provider: confirmed_provider,
                account_profile,
            } = response
            else {
                return Err(unconfirmed_profile_account());
            };
            if confirmed_provider != provider || account_profile != update.account_profile {
                return Err(unconfirmed_profile_account());
            }
            if let Some(status) = copy {
                // This validates placement/generation and persists the confirmed
                // receipt in one path. Never cache success before it accepts.
                registry.record_confirmed_account_copy(
                    &owner,
                    &expected_copy,
                    target_kind,
                    &binding.worker_machine_id,
                    &update.worker_kernel_id,
                    &account_profile,
                    status,
                )?;
            } else {
                // Preserve the existing metadata-only/legacy response contract.
                registry.update_materialization_status(
                    &owner,
                    provider,
                    &update.account_profile,
                    ProviderAccountMaterializationStatus {
                        copy: None,
                        target_kind,
                        target_ref: update.worker_kernel_id.clone(),
                        state: ProviderAccountMaterializationState::Materialized,
                        observed_at_ms: crate::session::unix_epoch_ms(),
                        last_error: None,
                    },
                )?;
            }
            Ok(account_profile)
        }
        .await;
        if result.is_err() {
            // A rejected receipt must not be reusable by the next profile change.
            // Keep the lease and queued work; only this installation attempt fails.
            registry.update_materialization_status(
                &owner,
                provider,
                &update.account_profile,
                ProviderAccountMaterializationStatus {
                    copy: None,
                    target_kind,
                    target_ref: update.worker_kernel_id.clone(),
                    state: ProviderAccountMaterializationState::Error,
                    observed_at_ms: crate::session::unix_epoch_ms(),
                    last_error: Some("selected account transfer failed; profile unchanged".into()),
                },
            )?;
        }
        result
    }
}

fn unconfirmed_profile_account() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "materialize remote profile account",
        message: "worker did not confirm the selected provider account; profile unchanged".into(),
    }
}

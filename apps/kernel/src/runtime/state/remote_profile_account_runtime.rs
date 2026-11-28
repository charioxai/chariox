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
        if crate::provider::canonical_provider_family(agent.provider()) == Some("claude") {
            return self
                .reinstall_remote_native_account(&dispatch.agent_id)
                .await;
        }
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

    /// Restore a lost tracked Claude copy for either a native launch or a prompt retry.
    pub(super) async fn reinstall_remote_native_account(
        &self,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if crate::provider::canonical_provider_family(agent.provider()) != Some("claude") {
            return Ok(());
        }
        // Row 577: the worker lost its copy. Forget the tracked copy and make it again.
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        if let Some(binding) = agent.remote_execution() {
            self.owned
                .provider_account_profiles
                .update_materialization_status(
                    &owner,
                    "claude",
                    agent.provider_account_profile(),
                    ProviderAccountMaterializationStatus {
                        target_kind: ProviderAccountMaterializationTargetKind::Copy,
                        target_ref: binding.worker_kernel_id.clone(),
                        state: ProviderAccountMaterializationState::Error,
                        observed_at_ms: crate::session::unix_epoch_ms(),
                        last_error: Some(WORKER_LOST_COPY.into()),
                    },
                )?;
        }
        self.ensure_agent_account_on_worker(agent_id).await
    }

    /// Row 577: a leased Claude agent runs from the worker's OWN copy of the account. The first
    /// time the worker needs it, the home makes one tracked account copy to that worker (the
    /// product account copy over the encrypted peer channel, tracked here as a `Copy`
    /// materialization and on the worker as its transferred profile) and never sends the token
    /// with a launch. Every later launch, prompt and relaunch finds the copy already there.
    /// Other providers keep their existing placement.
    pub(in crate::runtime::state) async fn ensure_agent_account_on_worker(
        &self,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if crate::provider::canonical_provider_family(agent.provider()) != Some("claude") {
            return Ok(());
        }
        let Some(binding) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let registry = self.owned.provider_account_profiles.clone();
        let profile = registry.get(&owner, "claude", agent.provider_account_profile())?;
        if profile.is_installed_at(
            crate::account_profile::ProviderAccountMaterializationTargetKind::Copy,
            &binding.worker_kernel_id,
        ) {
            return Ok(());
        }
        // A copy replacing one the worker lost is its own operation; until it finishes the
        // recorded loss stays, so its retries keep that operation.
        let recovery = lost_copy_generation(&profile.materializations, &binding.worker_kernel_id);
        let result = self
            .copy_claude_account_to_worker(
                agent_id,
                &profile.profile_id,
                &binding.worker_kernel_id,
                recovery,
            )
            .await;
        let installed = result.is_ok();
        if !installed && recovery.is_some() {
            return result.map(|_| ());
        }
        registry.update_materialization_status(
            &owner,
            "claude",
            &profile.profile_id,
            ProviderAccountMaterializationStatus {
                target_kind: ProviderAccountMaterializationTargetKind::Copy,
                target_ref: binding.worker_kernel_id.clone(),
                state: if installed {
                    ProviderAccountMaterializationState::Materialized
                } else {
                    ProviderAccountMaterializationState::Error
                },
                observed_at_ms: crate::session::unix_epoch_ms(),
                last_error: result
                    .as_ref()
                    .err()
                    .map(|_| "the account copy to the worker did not finish".to_string()),
            },
        )?;
        result.map(|_| ())
    }

    async fn copy_claude_account_to_worker(
        &self,
        agent_id: &str,
        home_profile_id: &str,
        worker_kernel_id: &str,
        recovery: Option<u64>,
    ) -> Result<(), DaemonError> {
        use crate::provider_account_copy::{
            failure, TRANSFER_SIGN_IN_REQUIRED, TRANSFER_TARGET_NOT_OWNED, TRANSFER_WORKER_OUTDATED,
        };
        let config = self.config_snapshot().await;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        // The existing Vault unlock card, held until the owner answers it; the guard closes the
        // Vault again after the copy when the unlock policy is "ask every time".
        let mut unlock = Some(
            self.ensure_vault_unlocked_for_agent(
                agent.session_id(),
                agent_id,
                "copy the Claude account to the worker",
            )
            .await?,
        );
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        // The owner's token comes first: no token, no copy. Without one the kernel runs the
        // official sign-in in the existing card and the copy goes on when the person finishes it.
        let mut signed_in = false;
        loop {
            let has_token = crate::provider::resolve_provider_account_credentials(
                &config,
                &owner,
                "claude",
                home_profile_id,
            )
            .ok()
            .and_then(|mut environment| environment.remove(crate::provider::CLAUDE_OAUTH_TOKEN_ENV))
            .is_some_and(|token| !token.trim().is_empty());
            if has_token {
                break;
            }
            if signed_in {
                return Err(failure(TRANSFER_SIGN_IN_REQUIRED));
            }
            // Release the unlock before the sign-in: its guard closes the Vault when it ends, and a
            // guard still held after the next unlock would close that one too.
            drop(unlock.take());
            if !self
                .sign_in_claude_for_agent(agent_id, home_profile_id)
                .await?
            {
                return Err(failure(TRANSFER_SIGN_IN_REQUIRED));
            }
            signed_in = true;
            // Storing the new token can close the Vault again (the "ask every time" policy), so
            // the unlock is taken again, with its own card, before the token is read.
            unlock = Some(
                self.ensure_vault_unlocked_for_agent(
                    agent.session_id(),
                    agent_id,
                    "copy the Claude account to the worker",
                )
                .await?,
            );
        }
        // Held until the copy returns.
        let _unlock = unlock;
        let discovery = crate::transport::relay_discovery::metadata_discovery_config(&config)
            .await
            .map_err(|_| failure(TRANSFER_TARGET_NOT_OWNED))?;
        let peer = crate::transport::relay_discovery::get_live_kernel(&discovery, worker_kernel_id)
            .await
            .map_err(|_| failure(TRANSFER_TARGET_NOT_OWNED))?;
        // One operation per (account, worker): a retry returns the saved receipt, so a copy the
        // worker already holds is never sent twice.
        let operation_id = lease_copy_operation_id(
            &config.daemon_id,
            home_profile_id,
            worker_kernel_id,
            recovery,
        );
        let request = crate::local::CopyProviderAccountToKernelRequest {
            provider: "claude".to_string(),
            account_profile: home_profile_id.to_string(),
            operation_id,
            target_kernel_id: worker_kernel_id.to_string(),
            target_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &peer.public_key,
            ),
            replace: false,
        };
        match self
            .copy_provider_account_to_kernel(crate::session::DEFAULT_LOCAL_USER_ID, request)
            .await
        {
            Ok(_) => Ok(()),
            Err(DaemonError::ProviderAccountCopy {
                code: "provider_account_copy_unauthorized",
                ..
            }) => Err(failure(TRANSFER_TARGET_NOT_OWNED)),
            Err(DaemonError::ProviderAccountCopy {
                code: "provider_account_copy_unsupported",
                ..
            }) => Err(failure(TRANSFER_WORKER_OUTDATED)),
            Err(DaemonError::ProviderAccountCopy {
                code: "provider_account_copy_profile",
                ..
            }) => Err(failure(TRANSFER_SIGN_IN_REQUIRED)),
            Err(error) => Err(error),
        }
    }

    /// The official Claude sign-in as the existing provider-login card, started on this kernel
    /// for the agent's Room, and awaited: the held prompt goes on by itself when the person
    /// finishes it. The person never runs a command for it. `true` once it succeeded.
    async fn sign_in_claude_for_agent(
        &self,
        agent_id: &str,
        home_profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let owner = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let started =
            crate::runtime::provider_auth_control::execute_start_provider_login_with_overwrite(
                self,
                &owner,
                crate::local::StartProviderLoginRequest {
                    provider: "claude".to_string(),
                    account_profile: home_profile_id.to_string(),
                    method: Some(
                        crate::runtime::claude_setup_token_login::CLAUDE_SETUP_TOKEN_METHOD
                            .to_string(),
                    ),
                },
                true,
            )
            .await?;
        let crate::local::LocalDaemonResponse::ProviderLoginStarted { login } = &started.response
        else {
            return Ok(false);
        };
        let id = format!(
            "provider-auth-recovery:{}",
            login.login_id.as_deref().unwrap_or_default()
        );
        self.wait_for_account_login(agent.session_id(), agent_id, &owner, &id, login)
            .await
    }
}

/// Row 577: what the home records when the worker answered that it no longer holds its copy.
const WORKER_LOST_COPY: &str = "the worker no longer holds the account copy";

/// The copy operation for this account on this worker. A retry of one copy keeps its id, so a
/// copy whose answer was lost is never sent twice; the copy that replaces one the worker lost is a
/// new operation, named by when the loss was recorded.
fn lease_copy_operation_id(
    home_kernel_id: &str,
    home_profile_id: &str,
    worker_kernel_id: &str,
    recovery: Option<u64>,
) -> String {
    use sha2::Digest;
    let mut input = format!("{home_kernel_id}\0{home_profile_id}\0{worker_kernel_id}");
    if let Some(recorded_at_ms) = recovery {
        input.push_str(&format!("\0recovery-{recorded_at_ms}"));
    }
    format!(
        "lease-{}",
        &format!("{:x}", sha2::Sha256::digest(input))[..32]
    )
}

/// When the home recorded that `worker_kernel_id` lost its copy, while that recovery has not
/// finished.
fn lost_copy_generation(
    materializations: &[ProviderAccountMaterializationStatus],
    worker_kernel_id: &str,
) -> Option<u64> {
    materializations
        .iter()
        .find(|status| {
            status.target_kind == ProviderAccountMaterializationTargetKind::Copy
                && status.target_ref == worker_kernel_id
                && status.state == ProviderAccountMaterializationState::Error
                && status.last_error.as_deref() == Some(WORKER_LOST_COPY)
        })
        .map(|status| status.observed_at_ms)
}

#[cfg(test)]
mod lease_copy_operation_tests {
    use super::*;

    fn copy_status(
        state: ProviderAccountMaterializationState,
        error: Option<&str>,
        at: u64,
    ) -> ProviderAccountMaterializationStatus {
        ProviderAccountMaterializationStatus {
            target_kind: ProviderAccountMaterializationTargetKind::Copy,
            target_ref: "worker".into(),
            state,
            observed_at_ms: at,
            last_error: error.map(str::to_string),
        }
    }

    fn operation_for(materializations: &[ProviderAccountMaterializationStatus]) -> String {
        lease_copy_operation_id(
            "home",
            "claude-1",
            "worker",
            lost_copy_generation(materializations, "worker"),
        )
    }

    // Review P2 (row 577): the worker lost the copy it had installed. Its record of the first copy
    // is immutable, so the copy that replaces it must be a new operation, or the worker replays the
    // old receipt and refuses it (`provider_account_copy_conflict`) on every later prompt.
    #[test]
    fn the_copy_after_the_worker_lost_its_copy_is_a_new_operation() {
        let first = operation_for(&[]);
        let lost = [copy_status(
            ProviderAccountMaterializationState::Error,
            Some(WORKER_LOST_COPY),
            1_000,
        )];
        assert_ne!(
            operation_for(&lost),
            first,
            "a recovery reused the completed copy's operation"
        );
        // A retry of that same recovery (its answer lost) keeps its operation.
        assert_eq!(operation_for(&lost), operation_for(&lost));
        // A second loss later is another new copy.
        let lost_again = [copy_status(
            ProviderAccountMaterializationState::Error,
            Some(WORKER_LOST_COPY),
            2_000,
        )];
        assert_ne!(operation_for(&lost_again), operation_for(&lost));
    }

    // A copy that did not finish (its answer may be lost) retries under the same operation, so the
    // worker answers from its receipt and nothing is installed twice.
    #[test]
    fn a_copy_that_did_not_finish_retries_under_the_same_operation() {
        let unfinished = [copy_status(
            ProviderAccountMaterializationState::Error,
            Some("the account copy to the worker did not finish"),
            1_000,
        )];
        assert_eq!(operation_for(&unfinished), operation_for(&[]));
        let other_worker = [ProviderAccountMaterializationStatus {
            target_ref: "another-worker".into(),
            ..copy_status(
                ProviderAccountMaterializationState::Error,
                Some(WORKER_LOST_COPY),
                1_000,
            )
        }];
        assert_eq!(operation_for(&other_worker), operation_for(&[]));
    }

    // The recovery path records the loss with the marker the next copy reads.
    #[test]
    fn the_recorded_loss_names_the_recovery_and_a_finished_copy_clears_it() {
        let lost = [copy_status(
            ProviderAccountMaterializationState::Error,
            Some(WORKER_LOST_COPY),
            7,
        )];
        assert_eq!(lost_copy_generation(&lost, "worker"), Some(7));
        let done = [copy_status(
            ProviderAccountMaterializationState::Materialized,
            None,
            8,
        )];
        assert_eq!(lost_copy_generation(&done, "worker"), None);
    }
}

//! Explicit account copy admission and the encrypted transport adapter.
use super::runtime_vault_unlock_state::{
    expand_vault_path, vault_unlock_request_lock, VaultUnlockGuard,
};
use super::*;
use crate::provider_account_copy::{
    failure, native_copy_credential_path, ProviderAccountCopyBinding,
    ProviderAccountCopyCredential, ProviderAccountCopyOutcome, ProviderAccountCopyReceipt,
    ProviderAccountCopyWait, PROVIDER_ACCOUNT_COPY_CAPABILITY,
    PROVIDER_ACCOUNT_NATIVE_COPY_CAPABILITY,
};

fn unlocked(config: &crate::config::DaemonConfig) -> Result<(), DaemonError> {
    if config.user_config.credential_vault.backend
        == crate::config::CredentialVaultBackend::CharioxEncrypted
        && !crate::secret::chariox_encrypted_vault_status(expand_vault_path(
            &config.user_config.credential_vault.path,
        ))
        .map_err(|_| failure("provider_account_copy_vault_unreadable"))?
        .unlocked
    {
        return Err(failure("provider_account_copy_vault_locked"));
    }
    Ok(())
}

impl KernelRuntimeState {
    pub(crate) async fn copy_provider_account_to_kernel(
        &self,
        caller: &str,
        request: crate::local::CopyProviderAccountToKernelRequest,
    ) -> Result<ProviderAccountCopyOutcome, DaemonError> {
        let config = self.config_snapshot().await;
        let enrollment = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| failure("provider_account_copy_unauthorized"))?;
        if ![
            crate::session::DEFAULT_LOCAL_USER_ID,
            enrollment.user_id.as_str(),
        ]
        .contains(&caller)
            || enrollment.user_id.is_empty()
            || enrollment.realm_id.is_empty()
            || enrollment.kernel_id.as_deref() != Some(config.daemon_id.as_str())
            || enrollment.kernel_public_key_thumbprint.as_deref()
                != Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(
                        &config.relay_public_key,
                    )
                    .as_str(),
                )
        {
            return Err(failure("provider_account_copy_unauthorized"));
        }
        let binding = ProviderAccountCopyBinding {
            operation_id: request.operation_id,
            provider: request.provider,
            source_profile_id: request.account_profile,
            source_kernel_id: config.daemon_id.clone(),
            source_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &config.relay_public_key,
            ),
            target_kernel_id: request.target_kernel_id,
            target_key_thumbprint: request.target_key_thumbprint,
            replace: request.replace,
        };
        binding.validate()?;
        let native_path = native_copy_credential_path(&binding.provider);
        let owner = self.provider_account_authority_owner_user_id(caller);
        let profile = self
            .provider_account_profile_registry()
            .get(&owner, &binding.provider, &binding.source_profile_id)
            .map_err(|_| failure("provider_account_copy_profile"))?;
        if profile.profile_id != binding.source_profile_id
            || profile.auth_state != crate::account_profile::ProviderAccountAuthState::Authenticated
            || (native_path.is_none()
                && !crate::provider::provider_account_credential_uses_vault(
                    &owner,
                    &binding.provider,
                    &profile.profile_id,
                )
                .map_err(|_| failure("provider_account_copy_profile"))?)
        {
            return Err(failure("provider_account_copy_profile"));
        }
        let enrolled =
            crate::managed_context::owner_authority::authorize_account_copy_peer(&config, &binding)
                .await?;
        // Discovery uses the authenticated owner's fresh visible target set.
        // A path to an admin socket is never peer enrollment or ownership proof.
        if !self.owned.relay_state.read().await.connected() {
            return Err(failure("provider_account_copy_unavailable"));
        }
        let discovery = crate::transport::relay_discovery::metadata_discovery_config(&config)
            .await
            .map_err(|_| failure("provider_account_copy_unavailable"))?;
        let peer = crate::transport::relay_discovery::get_live_kernel(
            &discovery,
            &binding.target_kernel_id,
        )
        .await
        .map_err(|_| failure("provider_account_copy_unavailable"))?;
        if peer.public_key != enrolled.relay_public_key
            || peer.kernel_id != binding.target_kernel_id
            || crate::runtime::terminal_pairings::public_key_thumbprint(&peer.public_key)
                != binding.target_key_thumbprint
        {
            return Err(failure("provider_account_copy_unauthorized"));
        }
        if let Some(copy) = self.existing_account_copy(&owner, &binding)? {
            return Ok(copy);
        }
        let capability = if native_path.is_some() {
            PROVIDER_ACCOUNT_NATIVE_COPY_CAPABILITY
        } else {
            PROVIDER_ACCOUNT_COPY_CAPABILITY
        };
        if !peer.capabilities.iter().any(|c| c == capability) {
            return Err(failure("provider_account_copy_unsupported"));
        }
        let target = ClientTarget {
            daemon_id: Some(peer.kernel_id.clone()),
            daemon_alias: None,
        };
        let probe = crate::transport::relay_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
            &config, &self.owned.relay_state, target, &peer.public_key,
            RelayPeerRequest::Ping { value: "provider-account-copy".into() }, Duration::from_secs(10),
        ).await.map_err(|_| failure("provider_account_copy_unavailable"))?;
        if !matches!(probe, RelayPeerResponse::Pong { daemon_id, relay_peer_protocol_version: Some(version), .. }
            if daemon_id == binding.target_kernel_id && version >= 215)
        {
            return Err(failure("provider_account_copy_unsupported"));
        }
        let credential = if let Some(path) = native_path {
            let materialization = self
                .provider_account_profile_registry()
                .export_managed_context_materialization(
                    &owner,
                    &binding.provider,
                    &profile.profile_id,
                )
                .map_err(|_| failure("provider_account_copy_profile"))?;
            let file = materialization
                .files
                .into_iter()
                .find(|file| file.relative_path == path)
                .ok_or_else(|| failure("provider_account_copy_profile"))?;
            ProviderAccountCopyCredential::new(file.contents_base64)
        } else {
            let _unlock = vault_unlock_request_lock(&expand_vault_path(
                &config.user_config.credential_vault.path,
            ))
            .lock_owned()
            .await;
            unlocked(&config)?;
            let selected = crate::provider::resolve_provider_account_credentials(
                &config,
                &owner,
                &binding.provider,
                &binding.source_profile_id,
            )
            .map_err(|error| crate::provider_account_copy::credential_read_failure(&error))?;
            let value = selected
                .iter()
                .find(|(name, _)| *name == crate::provider::CLAUDE_OAUTH_TOKEN_ENV)
                .map(|(_, value)| value.to_string())
                .ok_or_else(|| failure("provider_account_copy_profile"))?;
            ProviderAccountCopyCredential::new(value)
        };
        let target_label = peer
            .kernel_alias
            .or(peer.relay_alias)
            .unwrap_or_else(|| peer.kernel_id.clone());
        let send_copy = {
            let state = self.clone();
            let binding = binding.clone();
            move |resume_after_unlock| {
                let state = state.clone();
                let config = config.clone();
                let public_key = peer.public_key.clone();
                let binding = binding.clone();
                let credential = credential.clone();
                async move {
                    if resume_after_unlock {
                        let current = state.config_snapshot().await;
                        if current.relay_public_key != config.relay_public_key
                            || current
                                .cloud_relay
                                .as_ref()
                                .map(|p| (&p.user_id, &p.realm_id))
                                != config
                                    .cloud_relay
                                    .as_ref()
                                    .map(|p| (&p.user_id, &p.realm_id))
                        {
                            return Err(failure("provider_account_copy_unauthorized"));
                        }
                        crate::managed_context::owner_authority::authorize_account_copy_peer(
                            &current, &binding,
                        )
                        .await?;
                    }
                    crate::transport::relay_client::send_peer_request_to_known_kernel_via_relay_with_timeout(
                        &config, &state.owned.relay_state,
                        ClientTarget { daemon_id: Some(binding.target_kernel_id.clone()), daemon_alias: None }, &public_key,
                        RelayPeerRequest::InstallProviderAccountCopy { binding, credential, resume_after_unlock },
                        Duration::from_secs(90),
                    ).await.map_err(|_| failure("provider_account_copy_unavailable"))
                }
            }
        };
        match send_copy(false).await? {
            RelayPeerResponse::ProviderAccountCopyInstalled { receipt }
                if receipt == ProviderAccountCopyReceipt::installed(&binding) =>
            {
                Ok(ProviderAccountCopyOutcome::Installed(receipt))
            }
            RelayPeerResponse::ProviderAccountCopyTargetVaultLocked { expires_at_ms } => {
                let wait = ProviderAccountCopyWait {
                    operation_id: binding.operation_id.clone(),
                    source_kernel_id: binding.source_kernel_id.clone(),
                    target_kernel_id: binding.target_kernel_id.clone(),
                    target_label,
                    expires_at_ms,
                };
                self.wait_for_account_copy(owner, binding, wait.clone(), move || send_copy(true));
                self.notify_account_copy_progress();
                Ok(ProviderAccountCopyOutcome::Waiting(wait))
            }
            RelayPeerResponse::ProviderAccountCopyFailed { code } => {
                Err(install_peer_failure(&code))
            }
            _ => Err(failure("provider_account_copy_unsupported")),
        }
    }

    pub(crate) async fn install_provider_account_copy(
        &self,
        source_kernel: &str,
        source_key: &str,
        identity: &chariox_relay::protocol::RelayCallerIdentity,
        binding: ProviderAccountCopyBinding,
        credential: ProviderAccountCopyCredential,
        resume_after_unlock: bool,
    ) -> Result<RelayPeerResponse, DaemonError> {
        binding.validate()?;
        let config = self.config_snapshot().await;
        let profile = config
            .cloud_relay
            .as_ref()
            .ok_or_else(|| failure("provider_account_copy_unauthorized"))?;
        if profile.user_id.is_empty()
            || identity.user_id.as_deref() != Some(profile.user_id.as_str())
            || identity.realm_id != profile.realm_id
            || profile.realm_id.is_empty()
            || profile.machine_id.as_deref() != Some(config.host_machine_id.as_str())
            || profile
                .kernel_credential
                .as_deref()
                .is_none_or(str::is_empty)
            || profile.kernel_id.as_deref() != Some(config.daemon_id.as_str())
            || profile.kernel_public_key_thumbprint.as_deref()
                != Some(binding.target_key_thumbprint.as_str())
            || source_kernel != binding.source_kernel_id
            || (identity.subject_kind == chariox_relay::auth::RelaySubjectKind::Kernel
                && chariox_relay::protocol::canonical_peer_daemon_id(&identity.subject)
                    != Some(source_kernel))
            || crate::runtime::terminal_pairings::public_key_thumbprint(source_key)
                != binding.source_key_thumbprint
            || config.daemon_id != binding.target_kernel_id
            || crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)
                != binding.target_key_thumbprint
        {
            return Err(failure("provider_account_copy_unauthorized"));
        }
        let native = native_copy_credential_path(&binding.provider).is_some();
        if credential.value().is_empty() || (!native && credential.value().len() > 16 * 1024) {
            return Err(failure("invalid_request"));
        }
        let owner = self.provider_account_authority_owner_user_id(&profile.user_id);
        let profiles = self.provider_account_profile_registry().clone();
        if native {
            if binding.replace
                && profiles
                    .get(&owner, &binding.provider, &binding.target_profile_id())
                    .is_ok()
            {
                crate::runtime::provider_account_control::ensure_profile_idle(
                    self,
                    &profiles,
                    &owner,
                    &binding.provider,
                    &binding.target_profile_id(),
                )
                .map_err(|_| failure("provider_account_copy_conflict"))?;
            }
            let receipt = tokio::task::spawn_blocking(move || {
                let receipt =
                    profiles.install_account_copy(&config, &owner, &binding, &credential)?;
                // As with a managed-machine import, the native provider owns
                // validation. A missing executable does not undo the copy.
                let _ = crate::local::provider_requests::observe_provider_auth_status(
                    &profiles,
                    &owner,
                    &receipt.provider,
                    &receipt.account_profile,
                    &config,
                );
                Ok::<_, DaemonError>(receipt)
            })
            .await
            .map_err(|_| failure("provider_account_copy_install_failed"))??;
            self.with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
            self.record_room_list_change();
            return Ok(RelayPeerResponse::ProviderAccountCopyInstalled { receipt });
        }
        let vault_path = expand_vault_path(&config.user_config.credential_vault.path);
        let unlock = vault_unlock_request_lock(&vault_path).lock_owned().await;
        let encrypted = config.user_config.credential_vault.backend
            == crate::config::CredentialVaultBackend::CharioxEncrypted;
        let locked = match unlocked(&config) {
            Ok(()) => {
                encrypted
                    && crate::secret::chariox_vault_held_for_another(&vault_path, None)
                        .map_err(|_| failure("provider_account_copy_vault_unreadable"))?
            }
            Err(DaemonError::ProviderAccountCopy {
                code: "provider_account_copy_vault_locked",
                ..
            }) => true,
            Err(error) => return Err(error),
        };
        if let Some(expires_at_ms) = self
            .account_copy_unlock_gate(
                &profile.user_id,
                &binding,
                &vault_path,
                locked,
                resume_after_unlock,
            )
            .await?
        {
            return Ok(RelayPeerResponse::ProviderAccountCopyTargetVaultLocked { expires_at_ms });
        }
        let lease = if encrypted {
            VaultUnlockGuard::for_existing_unlock(
                vault_path,
                &format!("account-copy:{}", binding.operation_id),
            )
            .map_err(|_| failure("provider_account_copy_vault_unreadable"))?
            .ok_or_else(|| failure("provider_account_copy_vault_locked"))?
        } else {
            VaultUnlockGuard::not_required()
        };
        let result = tokio::task::spawn_blocking(move || {
            let _unlock = unlock;
            let _lease = lease;
            unlocked(&config)?;
            let receipt = profiles.install_account_copy(&config, &owner, &binding, &credential)?;
            // "Ask every time": the owner's unlock covered this one copy.
            let vault = &config.user_config.credential_vault;
            if vault.backend == crate::config::CredentialVaultBackend::CharioxEncrypted
                && vault.unlock_policy == crate::config::CredentialVaultUnlockPolicy::Always
            {
                crate::secret::lock_chariox_encrypted_vault(expand_vault_path(&vault.path))?;
                crate::secret::clear_vault_secret_process_cache()?;
            }
            Ok(receipt)
        })
        .await
        .map_err(|_| failure("provider_account_copy_install_failed"))?;
        if result.is_ok() {
            self.with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
            self.record_room_list_change();
        }
        result.map(|receipt| RelayPeerResponse::ProviderAccountCopyInstalled { receipt })
    }
}

pub(super) fn install_peer_failure(code: &str) -> DaemonError {
    failure(match code {
        "provider_account_copy_vault_locked" => "provider_account_copy_target_vault_locked",
        "provider_account_copy_vault_unreadable" => "provider_account_copy_target_vault_unreadable",
        "provider_account_copy_credential_unreadable" => {
            "provider_account_copy_target_credential_unreadable"
        }
        "provider_account_copy_unauthorized" => "provider_account_copy_unauthorized",
        "provider_account_copy_cancelled" => "provider_account_copy_cancelled",
        "provider_account_copy_cli_missing" => "provider_account_copy_cli_missing",
        "provider_account_copy_replace_differs" => "provider_account_copy_replace_differs",
        "provider_account_copy_conflict" => "provider_account_copy_conflict",
        "provider_account_copy_rejected" => "provider_account_copy_rejected",
        "provider_account_copy_verification_failed" => "provider_account_copy_verification_failed",
        _ => "provider_account_copy_install_failed",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Vault state a client shows for a kernel is the state the kernel's copy
    /// admission sees: both read the same status of the configured path.
    #[test]
    fn copy_admission_sees_the_vault_state_the_status_reports() {
        let root = crate::test_support::TestWorktree::new("copy-admission-vault-state");
        let vault = root.path().join("vault.json");
        let mut config = crate::config::DaemonConfig::for_tests();
        config.user_config.credential_vault.backend =
            crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = vault.to_string_lossy().into_owned();
        let status = || {
            crate::secret::chariox_encrypted_vault_status(&config.user_config.credential_vault.path)
                .unwrap()
        };
        let admission = || match unlocked(&config) {
            Ok(()) => true,
            Err(DaemonError::ProviderAccountCopy {
                code: "provider_account_copy_vault_locked",
                ..
            }) => false,
            Err(other) => panic!("a readable Vault must not fail otherwise: {other}"),
        };

        // No Vault yet: waiting for a passphrase, never shown or treated as open.
        assert!(status().awaiting_passphrase);
        assert!(!status().unlocked);
        assert!(!admission());

        crate::secret::create_chariox_encrypted_vault_for_test(&vault, "fixture-pass").unwrap();
        assert!(!status().unlocked);
        assert!(!admission());

        crate::secret::unlock_chariox_encrypted_vault(
            &vault,
            "fixture-pass",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        assert!(status().unlocked);
        assert!(admission());

        crate::secret::lock_chariox_encrypted_vault(&vault).unwrap();
        assert!(!status().unlocked);
        assert!(!admission());
    }
}

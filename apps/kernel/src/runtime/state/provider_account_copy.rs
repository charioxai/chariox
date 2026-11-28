//! Explicit account copy admission and the encrypted transport adapter.
use super::runtime_vault_unlock_state::{
    expand_vault_path, vault_unlock_request_lock, VaultUnlockGuard,
};
use super::*;
use crate::provider_account_copy::{
    failure, AccountCopyInstall, ProviderAccountCopyBinding, ProviderAccountCopyCredential,
    ProviderAccountCopyOutcome, ProviderAccountCopyReceipt, ProviderAccountCopyWait,
    MAX_COPY_CREDENTIAL_BYTES, PROVIDER_ACCOUNT_COPY_CAPABILITY,
};

pub(super) fn unlocked(config: &crate::config::DaemonConfig) -> Result<(), DaemonError> {
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

/// A kernel enrolled by its owner holds a kernel credential and its pinned identity. A kernel
/// that enrolled with its machine credential alone holds no kernel credential, and the identity
/// Cloud pinned for its machine is the relay key it runs with; the install path below verifies
/// that key against the binding.
pub(super) fn receiver_enrolled(
    profile: &crate::config::PersistedCloudRelayProfile,
    daemon_id: &str,
    target_key_thumbprint: &str,
) -> bool {
    let has = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.is_empty());
    let kernel = has(&profile.kernel_credential)
        && profile.kernel_id.as_deref() == Some(daemon_id)
        && profile.kernel_public_key_thumbprint.as_deref() == Some(target_key_thumbprint);
    let machine_enrolled = profile.kernel_credential.is_none()
        && profile.kernel_id.is_none()
        && profile.kernel_public_key_thumbprint.is_none()
        && has(&profile.machine_credential);
    kernel || machine_enrolled
}

#[derive(Clone, Copy)]
enum CopyCompletion {
    Installed,
    MayWait,
}

impl KernelRuntimeState {
    /// Internal admission callers need a completed install, never a pending copy.
    pub(crate) async fn copy_provider_account_to_kernel(
        &self,
        caller: &str,
        request: crate::local::CopyProviderAccountToKernelRequest,
    ) -> Result<ProviderAccountCopyReceipt, DaemonError> {
        match self
            .copy_account(caller, request, CopyCompletion::Installed)
            .await?
        {
            ProviderAccountCopyOutcome::Installed(receipt) => Ok(receipt),
            ProviderAccountCopyOutcome::Waiting(_) => {
                Err(failure("provider_account_copy_target_vault_locked"))
            }
        }
    }

    pub(crate) async fn begin_provider_account_copy_to_kernel(
        &self,
        caller: &str,
        request: crate::local::CopyProviderAccountToKernelRequest,
    ) -> Result<ProviderAccountCopyOutcome, DaemonError> {
        self.copy_account(caller, request, CopyCompletion::MayWait)
            .await
    }

    async fn copy_account(
        &self,
        caller: &str,
        request: crate::local::CopyProviderAccountToKernelRequest,
        completion: CopyCompletion,
    ) -> Result<ProviderAccountCopyOutcome, DaemonError> {
        let config = self.config_snapshot().await;
        if caller != crate::session::DEFAULT_LOCAL_USER_ID
            && config
                .cloud_relay
                .as_ref()
                .is_none_or(|profile| profile.user_id != caller)
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
        let owner = self.provider_account_authority_owner_user_id(caller);
        let profile = self
            .provider_account_profile_registry()
            .get(&owner, &binding.provider, &binding.source_profile_id)
            .map_err(|_| failure("provider_account_copy_profile"))?;
        if profile.profile_id != binding.source_profile_id
            || profile.auth_state != crate::account_profile::ProviderAccountAuthState::Authenticated
        {
            return Err(failure("provider_account_copy_profile"));
        }
        if binding.target_kernel_id == config.daemon_id {
            if caller != crate::session::DEFAULT_LOCAL_USER_ID
                && config
                    .cloud_relay
                    .as_ref()
                    .is_none_or(|profile| profile.user_id != caller)
            {
                return Err(failure("provider_account_copy_unauthorized"));
            }
            let profiles = self.provider_account_profile_registry().clone();
            let receipt = tokio::task::spawn_blocking(move || {
                profiles.install_account_link(&owner, &binding)
            })
            .await
            .map_err(|_| failure("provider_account_copy_install_failed"))??;
            self.with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
                .await;
            self.record_room_list_change();
            return Ok(ProviderAccountCopyOutcome::Installed(receipt));
        }
        if binding.provider == "claude"
            && !crate::provider::provider_account_credential_uses_vault(
                &owner,
                &binding.provider,
                &self
                    .provider_account_profile_registry()
                    .credential_source_profile_id(&owner, &binding.provider, &profile.profile_id)?,
            )
            .map_err(|_| failure("provider_account_copy_profile"))?
        {
            return Err(failure("provider_account_copy_profile"));
        }
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
            if matches!(
                (&completion, &copy),
                (
                    CopyCompletion::Installed,
                    ProviderAccountCopyOutcome::Waiting(_)
                )
            ) {
                return Err(failure("provider_account_copy_target_vault_locked"));
            }
            return Ok(copy);
        }
        if !peer
            .capabilities
            .iter()
            .any(|c| c == PROVIDER_ACCOUNT_COPY_CAPABILITY)
            || !crate::provider_account_copy::kernel_handles_copy_of(
                &binding.provider,
                &peer.capabilities,
            )
        {
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
        let credential = {
            let owner = owner.clone();
            let _unlock = vault_unlock_request_lock(&expand_vault_path(
                &config.user_config.credential_vault.path,
            ))
            .lock_owned()
            .await;
            if binding.provider == "claude" {
                unlocked(&config)?;
            }
            let profiles = self.provider_account_profile_registry().clone();
            let source_config = config.clone();
            let source_binding = binding.clone();
            tokio::task::spawn_blocking(move || {
                profiles.export_account_copy_credential(
                    &source_config,
                    &owner,
                    &source_binding.provider,
                    &source_binding.source_profile_id,
                )
            })
            .await
            .map_err(|_| failure("provider_account_copy_install_failed"))??
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
                if matches!(completion, CopyCompletion::Installed) {
                    return Err(failure("provider_account_copy_target_vault_locked"));
                }
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
                Err(failure(match code.as_str() {
                    "provider_account_copy_unauthorized" => "provider_account_copy_unauthorized",
                    // The target answered, so the Vault that is closed is the receiving one.
                    "provider_account_copy_vault_locked" => {
                        "provider_account_copy_target_vault_locked"
                    }
                    "provider_account_copy_vault_unreadable" => {
                        "provider_account_copy_target_vault_unreadable"
                    }
                    "provider_account_copy_credential_unreadable" => {
                        "provider_account_copy_target_credential_unreadable"
                    }
                    "provider_account_copy_cli_missing" => "provider_account_copy_cli_missing",
                    "provider_account_copy_replace_differs" => {
                        "provider_account_copy_replace_differs"
                    }
                    "provider_account_copy_conflict" => "provider_account_copy_conflict",
                    "provider_account_copy_rejected" => "provider_account_copy_rejected",
                    "provider_account_copy_verification_failed" => {
                        "provider_account_copy_verification_failed"
                    }
                    _ => "provider_account_copy_install_failed",
                }))
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
            || !receiver_enrolled(profile, &config.daemon_id, &binding.target_key_thumbprint)
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
        if credential.value().is_empty() || credential.value().len() > MAX_COPY_CREDENTIAL_BYTES {
            return Err(failure("invalid_request"));
        }
        let owner = self.provider_account_authority_owner_user_id(&profile.user_id);
        let profiles = self.provider_account_profile_registry().clone();
        let vault_path = expand_vault_path(&config.user_config.credential_vault.path);
        let unlock = vault_unlock_request_lock(&vault_path).lock_owned().await;
        // An OpenCode login lives only in its native store: its copy neither needs
        // nor changes the Vault.
        let uses_vault = binding.provider != "opencode";
        let lease = if uses_vault {
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
                return Ok(RelayPeerResponse::ProviderAccountCopyTargetVaultLocked {
                    expires_at_ms,
                });
            }
            if encrypted {
                VaultUnlockGuard::for_existing_unlock(
                    vault_path,
                    &format!("account-copy:{}", binding.operation_id),
                )
                .map_err(|_| failure("provider_account_copy_vault_unreadable"))?
                .ok_or_else(|| failure("provider_account_copy_vault_locked"))?
            } else {
                VaultUnlockGuard::not_required()
            }
        } else {
            VaultUnlockGuard::not_required()
        };
        let result = tokio::task::spawn_blocking(move || {
            let _unlock = unlock;
            let _lease = lease;
            if uses_vault {
                unlocked(&config)?;
            }
            let installed =
                profiles.install_account_copy(&config, &owner, &binding, &credential)?;
            if !uses_vault {
                if let AccountCopyInstall::Fresh(receipt) = &installed {
                    // The copy is verified; this records the services the login holds.
                    let _ = crate::local::provider_requests::observe_provider_auth_status(
                        &profiles,
                        &owner,
                        &receipt.provider,
                        &receipt.account_profile,
                        &config,
                    );
                }
                return Ok(installed);
            }

            // "Ask every time": the owner's unlock covered this one copy.
            let vault = &config.user_config.credential_vault;
            if vault.backend == crate::config::CredentialVaultBackend::CharioxEncrypted
                && vault.unlock_policy == crate::config::CredentialVaultUnlockPolicy::Always
            {
                crate::secret::lock_chariox_encrypted_vault(expand_vault_path(&vault.path))?;
                crate::secret::clear_vault_secret_process_cache()?;
            }
            Ok::<_, DaemonError>(installed)
        })
        .await
        .map_err(|_| failure("provider_account_copy_install_failed"))?;
        let receipt = match result? {
            AccountCopyInstall::Replay(receipt) => {
                return Ok(RelayPeerResponse::ProviderAccountCopyInstalled { receipt })
            }
            AccountCopyInstall::Fresh(receipt) => receipt,
        };
        self.with_app_side_effect(|app| app.invalidate_provider_catalog_cache())
            .await;
        self.record_room_list_change();
        let owner =
            self.provider_account_authority_owner_user_id(crate::session::DEFAULT_LOCAL_USER_ID);
        let profile = self.provider_account_profile_registry().get(
            &owner,
            &receipt.provider,
            &receipt.account_profile,
        )?;
        self.notify_provider_account_sign_in(&profile, true).await?;
        Ok(RelayPeerResponse::ProviderAccountCopyInstalled { receipt })
    }
}

pub(super) fn install_peer_failure(code: &str) -> DaemonError {
    match code {
        "provider_account_copy_vault_locked" => {
            failure("provider_account_copy_target_vault_locked")
        }
        "provider_account_copy_vault_unreadable" => {
            failure("provider_account_copy_target_vault_unreadable")
        }
        "provider_account_copy_credential_unreadable" => {
            failure("provider_account_copy_target_credential_unreadable")
        }
        _ => super::provider_account_copy_offer::copy_peer_failure(code),
    }
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

#[cfg(test)]
mod receiver_enrollment_tests {
    use super::receiver_enrolled;
    use crate::config::PersistedCloudRelayProfile;

    const THUMB: &str = "aa";
    fn kernel() -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            kernel_id: Some("kernel-a".into()),
            kernel_credential: Some("kcred".into()),
            kernel_public_key_thumbprint: Some(THUMB.into()),
            ..Default::default()
        }
    }
    fn rented() -> PersistedCloudRelayProfile {
        PersistedCloudRelayProfile {
            machine_credential: Some("mcred_x".into()),
            ..Default::default()
        }
    }

    #[test]
    fn an_owner_enrolled_kernel_is_admitted_only_with_its_own_identity() {
        assert!(receiver_enrolled(&kernel(), "kernel-a", THUMB));
        assert!(!receiver_enrolled(&kernel(), "kernel-b", THUMB));
        assert!(!receiver_enrolled(&kernel(), "kernel-a", "bb"));
        let mut empty = kernel();
        empty.kernel_credential = Some(String::new());
        assert!(!receiver_enrolled(&empty, "kernel-a", THUMB));
    }

    #[test]
    fn a_rented_machine_with_only_its_machine_credential_is_admitted() {
        assert!(receiver_enrolled(&rented(), "kernel-r", THUMB));
    }

    #[test]
    fn an_unenrolled_or_half_enrolled_profile_is_refused() {
        assert!(!receiver_enrolled(
            &PersistedCloudRelayProfile::default(),
            "k",
            THUMB
        ));
        let mut empty = rented();
        empty.machine_credential = Some(String::new());
        assert!(!receiver_enrolled(&empty, "k", THUMB));
        let mut half = rented();
        half.kernel_id = Some("kernel-r".into());
        assert!(!receiver_enrolled(&half, "kernel-r", THUMB));
    }
}

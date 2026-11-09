//! MP-08/MP-10/MP-11: Vault retirement and autonomous environment lifecycle.
use super::*;

impl KernelRuntimeState {
    // Called only after successful provisioning with absent old container/home,
    // no saved state and no source slice. Metadata-only reset is insufficient.
    pub(crate) async fn reset_fresh_slice_observation_environment(
        &self,
        slice: &crate::slice::SliceRecord,
    ) -> Result<(), DaemonError> {
        if let Some(room) = slice
            .environment_session_id
            .as_deref()
            .filter(|room| self.owned.session_store.get_session(room).is_ok())
        {
            self.room_browser_controller_command(
                room,
                Command::ClearSecretObservation {
                    disposition: Disposition::ResetEnvironment,
                },
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn revoke_vault_observation_values(
        &self,
        key: &str,
    ) -> Result<(), DaemonError> {
        self.revoke_kernel_browser_observation_values(key).await?;
        let rooms = self.owned.session_store.list_all_sessions();
        for session in rooms {
            if self
                .owned
                .room_secret_observations
                .uses_vault_key(session.id(), key)?
            {
                self.room_browser_controller_command(
                    session.id(),
                    Command::ClearSecretObservation {
                        disposition: Default::default(),
                    },
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Drop deleted-key provenance in every scope; retain masks while another
    /// Vault key still owns the same bytes.
    pub(in crate::runtime::state) async fn drop_protected_value(
        &self,
        owner: &str,
        key: &str,
        value: &str,
        retained: bool,
    ) -> Result<(), DaemonError> {
        let protection = &self.owned.kernel_browser_secret_observations;
        let mut scopes = self
            .owned
            .kernel_browser_host
            .profile_keys()
            .map_err(super::super::kernel_browser_runtime::host_error)?;
        let own = crate::runtime::kernel_browser_host::KernelBrowserHost::profile_key(owner);
        if !scopes.contains(&own) {
            scopes.push(own);
        }
        for scope in scopes {
            let _barrier = protection.barrier(&scope)?.write_owned().await;
            protection.drop_value(&scope, key, value, retained)?;
        }
        for session in self.owned.session_store.list_all_sessions() {
            self.owned
                .room_secret_observations
                .drop_value(session.id(), key, value, retained)?;
        }
        Ok(())
    }

    pub(in crate::runtime::state) async fn room_secret_input_service(
        &self,
        room: &str,
        credential: &str,
    ) -> Result<
        (
            crate::secret::RuntimeSecretService,
            tokio::sync::OwnedRwLockReadGuard<()>,
        ),
        DaemonError,
    > {
        self.scoped_secret_input_service(&self.owned.room_secret_observations, room, credential)
            .await
    }

    // MD-5: Room and user-domain input share the Vault lifecycle lock and
    // authoritative metadata reload; only their observation scope differs.
    pub(in crate::runtime::state) async fn scoped_secret_input_service(
        &self,
        protection: &RoomSecretObservations,
        scope: &str,
        credential: &str,
    ) -> Result<
        (
            crate::secret::RuntimeSecretService,
            tokio::sync::OwnedRwLockReadGuard<()>,
        ),
        DaemonError,
    > {
        let guard = self
            .owned
            .room_secret_observations
            .vault_lifecycle
            .clone()
            .read_owned()
            .await;
        // Metadata and Vault configuration must be authoritative after every
        // unlock, approval and lifecycle wait. Keep this guard through insertion.
        let service = self.home_runtime_secret_service()?;
        protection.register_credential_source(scope, service.credential_vault_key(credential)?)?;
        Ok((service, guard))
    }

    /// Holds off every Vault value and registry write, as a secret fill does.
    pub(in crate::runtime::state) async fn vault_lifecycle_read_guard(
        &self,
    ) -> tokio::sync::OwnedRwLockReadGuard<()> {
        self.owned
            .room_secret_observations
            .vault_lifecycle
            .clone()
            .read_owned()
            .await
    }

    pub(crate) async fn vault_observation_mutation_guard(
        &self,
    ) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.owned
            .room_secret_observations
            .vault_lifecycle
            .clone()
            .write_owned()
            .await
    }

    pub(crate) async fn upsert_observed_credential_metadata(
        &self,
        registry: &crate::credential::CharioxCredentialRegistry,
        credential: crate::config::UserCredentialConfig,
    ) -> Result<(crate::config::UserCredentialConfig, std::path::PathBuf), DaemonError> {
        let _guard = self.vault_observation_mutation_guard().await;
        registry.validate_upsert(&credential)?;
        if let Some(previous) = registry.get(&credential.id)? {
            if previous.source != credential.source {
                if let crate::config::UserCredentialSourceConfig::Vault { key } = previous.source {
                    self.revoke_vault_observation_values(&key).await?;
                }
            }
        }
        registry.upsert(credential)
    }

    pub(crate) async fn remove_observed_credential_metadata(
        &self,
        registry: &crate::credential::CharioxCredentialRegistry,
        id: &str,
    ) -> Result<(crate::config::UserCredentialConfig, std::path::PathBuf), DaemonError> {
        let _guard = self.vault_observation_mutation_guard().await;
        if let Some(credential) = registry.get(id)? {
            if let crate::config::UserCredentialSourceConfig::Vault { key } = credential.source {
                self.revoke_vault_observation_values(&key).await?;
            }
        }
        registry.remove(id)
    }

    pub(in crate::runtime::state) async fn upsert_observed_vault_credential(
        &self,
        service: &crate::secret::RuntimeSecretService,
        registry: &crate::credential::CharioxCredentialRegistry,
        credential: crate::config::UserCredentialConfig,
        secret: &str,
        overwrite: bool,
    ) -> Result<crate::secret::VaultCredentialUpsertResult, DaemonError> {
        let _guard = self.vault_observation_mutation_guard().await;
        self.upsert_vault_credential_locked(service, registry, credential, secret, overwrite)
            .await
    }

    /// The owner's hand-off save. The owner may replace a Vault value that has no record,
    /// but never an existing credential record: only the same browser-only record is refreshed.
    /// The check and the write share one lifecycle guard.
    pub(in crate::runtime::state) async fn save_owner_handoff_credential(
        &self,
        service: &crate::secret::RuntimeSecretService,
        registry: &crate::credential::CharioxCredentialRegistry,
        mut credential: crate::config::UserCredentialConfig,
        secret: &str,
    ) -> Result<(), DaemonError> {
        let _guard = self.vault_observation_mutation_guard().await;
        if let Some(previous) = registry.get(&credential.id)? {
            if previous.source != credential.source
                || previous.allowed_hosts != credential.allowed_hosts
                || previous.allowed_uses != credential.allowed_uses
                || previous.injection != credential.injection
            {
                return Err(DaemonError::LocalTransport {
                    operation: "credential_vault_upsert",
                    message: format!(
                        "credential `{}` already exists for another use; save under a different name",
                        credential.id
                    ),
                });
            }
            credential.description = previous.description;
            credential.metadata = previous.metadata;
        }
        self.upsert_vault_credential_locked(service, registry, credential, secret, true)
            .await
            .map(|_| ())
    }

    /// The storage step of `upsert_observed_vault_credential`. The caller holds the Vault
    /// lifecycle write guard, so the registry it reads here is the one it writes.
    pub(in crate::runtime::state) async fn upsert_vault_credential_locked(
        &self,
        service: &crate::secret::RuntimeSecretService,
        registry: &crate::credential::CharioxCredentialRegistry,
        credential: crate::config::UserCredentialConfig,
        secret: &str,
        overwrite: bool,
    ) -> Result<crate::secret::VaultCredentialUpsertResult, DaemonError> {
        self.upsert_vault_credential_locked_for_agent(
            service, registry, credential, secret, overwrite, None,
        )
        .await
    }

    pub(in crate::runtime::state) async fn upsert_vault_credential_locked_for_agent(
        &self,
        service: &crate::secret::RuntimeSecretService,
        registry: &crate::credential::CharioxCredentialRegistry,
        credential: crate::config::UserCredentialConfig,
        secret: &str,
        overwrite: bool,
        context: Option<&super::super::vault_use::VaultAgentContext>,
    ) -> Result<crate::secret::VaultCredentialUpsertResult, DaemonError> {
        let (credential, secret) =
            crate::secret::normalize_vault_credential_input(credential, secret)?;
        let secret = secret.as_str();
        registry.validate_upsert(&credential)?;
        let previous = registry.get(&credential.id)?;
        // A new handle must not take over a Vault slot that holds a value: storing would
        // replace the value of the handle that uses it, or one that was stored or kept
        // without a record. `overwrite` is the caller's explicit choice to replace the latter.
        if previous.is_none() {
            if let crate::config::UserCredentialSourceConfig::Vault { key } = &credential.source {
                if let Some(holder) = registry.other_handle_using_vault_key(key, &credential.id)? {
                    return Err(DaemonError::LocalTransport {
                        operation: "credential_vault_upsert",
                        message: format!(
                            "Vault key `{key}` already holds the value of credential `{holder}`"
                        ),
                    });
                }
                if !overwrite && service.vault_slot_holds_value(key)? {
                    return Err(DaemonError::LocalTransport {
                        operation: "credential_vault_upsert",
                        message: format!(
                            "Vault key `{key}` already holds a value; pass overwrite=true to replace it"
                        ),
                    });
                }
            }
        }
        // Rejecting an existing handle or an empty value must not revoke unrelated observations.
        if (overwrite || previous.is_none()) && !secret.is_empty() {
            if let crate::config::UserCredentialSourceConfig::Vault { key } = &credential.source {
                self.revoke_vault_observation_values(key).await?;
                if let Some(previous) = previous {
                    if let crate::config::UserCredentialSourceConfig::Vault { key } =
                        previous.source
                    {
                        self.revoke_vault_observation_values(&key).await?;
                    }
                }
            }
        }
        // MP-08 / MP-10 / MP-11: runtime creation stamps its authoritative
        // Room into metadata. A value becomes private (scrub-only) when stored,
        // before any later browser insertion; prior public echoes are retired now.
        let room = credential
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.session_id.clone());
        let resource_metadata = credential.clone();
        let key = match &credential.source {
            crate::config::UserCredentialSourceConfig::Vault { key } => Some(key.clone()),
            _ => None,
        };
        self.with_authorized_app_side_effect(|_| {
            if let Some(context) = context {
                self.recheck_vault_agent(context)?;
            }
            self.with_forwarded_binding_operation(|| {
                let _history = self.owned.operational_history_store.lock_public_history()?;
                let result = service.upsert_vault_backed_credential_with_secret(
                    registry, credential, secret, overwrite,
                )?;
                if let Some(room) = room {
                    let source = self
                        .owned
                        .room_secret_observations
                        .register_credential_source(&room, key.as_deref());
                    // Protect the value even if persisting its provenance fails.
                    let protection = self
                        .owned
                        .room_secret_observations
                        .register_stored_with_locked_public_history(&room, secret);
                    source?;
                    protection?;
                }
                self.record_vault_resource(&resource_metadata)?;
                Ok(result)
            })
        })
        .await
    }
}

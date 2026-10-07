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
        self.owned
            .room_secret_observations
            .register_credential_source(room, service.credential_vault_key(credential)?)?;
        Ok((service, guard))
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
        crate::credential::validate_credential_registration(&credential)?;
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
        crate::credential::validate_credential_registration(&credential)?;
        let previous = registry.get(&credential.id)?;
        // A new handle can still overwrite an existing Vault key. Rejecting an
        // existing handle or an empty value must not revoke unrelated observations.
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
        // Room into metadata. A value becomes private when stored, before any
        // later browser insertion; previously public echoes must be retired now.
        let room = credential
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.session_id.clone());
        let key = match &credential.source {
            crate::config::UserCredentialSourceConfig::Vault { key } => Some(key.clone()),
            _ => None,
        };
        self.with_authorized_app_side_effect(|_| {
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
                        .register_with_locked_public_history(&room, secret);
                    source?;
                    protection?;
                }
                Ok(result)
            })
        })
        .await
    }
}

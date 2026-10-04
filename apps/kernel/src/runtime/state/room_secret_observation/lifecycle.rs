//! MP-08/MP-10/MP-11: Vault revocation and explicit legacy Room recovery.
use super::*;

impl KernelRuntimeState {
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
                        clear_unknown: false,
                    },
                )
                .await?;
            }
        }
        Ok(())
    }

    pub(in crate::runtime::state) async fn recover_unknown_room_observations(
        &self,
        room: &str,
    ) -> Result<&'static str, DaemonError> {
        let owner = self
            .owned
            .session_store
            .get_session(room)?
            .owner_user_id()
            .to_string();
        let id = format!(
            "vault-observation-recovery-{}",
            crate::session::unix_epoch_ms()
        );
        let interaction = crate::session::RuntimeInteraction::for_kernel_operation(
            &id, format!("room-observation-recovery:{room}"), "Recover Room observations",
            "Remove prior secrets from all browser pages and desktop controls first. Confirm only after the environment is clean. Prior history and cached artifacts will stay withheld.",
            vec![
                crate::session::RuntimeInteractionChoice::new("clear_observations", "I removed prior secrets; restore fresh observations", "clear_observations", Some(crate::session::RuntimeInteractionChoiceStyle::Danger)),
                crate::session::RuntimeInteractionChoice::new("dismiss", "Keep observations protected", "dismiss", Some(crate::session::RuntimeInteractionChoiceStyle::Secondary)),
            ],
        );
        let rx = self
            .create_kernel_operation_interaction(room, &owner, interaction)
            .await?;
        let resolution = match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
            Ok(Ok(resolution)) => resolution,
            Ok(Err(_)) => return Err(protection_error()),
            Err(_) => {
                self.timeout_runtime_interaction(room, &id).await?;
                return Ok("dismissed");
            }
        };
        if resolution.choice_id.as_deref() == Some("clear_observations") {
            let _guard = self.owned.room_secret_observations.vault_lifecycle.clone().try_write_owned().map_err(|_| DaemonError::LocalTransport {
                operation: "room.observation.recovery", message: "Finish pending credential operations, remove prior secrets, and retry Room recovery.".into(),
            })?;
            self.room_browser_controller_command(
                room,
                Command::ClearSecretObservation {
                    clear_unknown: true,
                },
            )
            .await?;
            Ok("observations_cleared")
        } else {
            Ok("dismissed")
        }
    }

    pub(in crate::runtime::state) async fn track_room_vault_key(
        &self,
        room: &str,
        service: &crate::secret::RuntimeSecretService,
        credential: &str,
    ) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, DaemonError> {
        let guard = self
            .owned
            .room_secret_observations
            .vault_lifecycle
            .clone()
            .read_owned()
            .await;
        self.owned
            .room_secret_observations
            .register_credential_source(room, service.credential_vault_key(credential)?)?;
        Ok(guard)
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
        service.upsert_vault_backed_credential_with_secret(registry, credential, secret, overwrite)
    }
}

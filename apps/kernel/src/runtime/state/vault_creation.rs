//! The owner's own client creates this kernel's Vault, and seals its key for another kernel's.
use super::runtime_vault_unlock_state::{expand_vault_path, vault_unlock_request_lock};
use super::vault_unlock_request::{
    owner_client_lease, refusal, vault_locked_error, wording, EXISTS, NOT_CHARIOX_ENCRYPTED,
};
use super::vault_unlock_requests::notify_vault_unlocked;
use super::*;
use crate::config::CredentialVaultBackend;
use crate::local::{
    CreateCredentialVaultRequest, CredentialVaultOrigin, SealCredentialVaultKeyRequest,
};
use std::path::PathBuf;

impl KernelRuntimeState {
    fn chariox_vault_path(
        &self,
    ) -> Result<(PathBuf, crate::config::UserCredentialVaultConfig), DaemonError> {
        let config = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        if config.backend != CredentialVaultBackend::CharioxEncrypted {
            return Err(refusal(
                NOT_CHARIOX_ENCRYPTED,
                wording::NOT_CHARIOX_ENCRYPTED,
                false,
            ));
        }
        Ok((expand_vault_path(&config.path), config))
    }

    /// `CreateCredentialVault`: a new Vault, unlocked for the owner's lease. It never replaces one.
    pub(crate) async fn create_credential_vault_for_owner(
        &self,
        owner: &str,
        request: CreateCredentialVaultRequest,
    ) -> Result<crate::secret::CharioxVaultUnlockStatus, DaemonError> {
        let (path, config) = self.chariox_vault_path()?;
        let _mutation = vault_unlock_request_lock(&path).lock_owned().await;
        if path.exists() {
            return Err(refusal(EXISTS, wording::EXISTS, false));
        }
        let lease = owner_client_lease(&config);
        let status = match request.origin {
            CredentialVaultOrigin::Passphrase { passphrase } => {
                self.unlock_vault_counted(owner, &path, passphrase.expose_secret(), lease, true)
                    .await?
            }
            CredentialVaultOrigin::Seed { seed } => {
                let kernel = self.owned.config_projection.snapshot();
                let created = crate::secret::create_vault_from_key_seed(
                    &path,
                    &seed,
                    &kernel.daemon_id,
                    &kernel.relay_private_key,
                    lease,
                )?;
                if let Err(error) = self.pin_critical_approval_verifier_after_unlock(&path) {
                    crate::secret::lock_chariox_encrypted_vault(&path)?;
                    crate::secret::clear_vault_secret_process_cache()?;
                    return Err(error);
                }
                created
            }
        };
        if lease != crate::secret::VaultUnlockLease::SingleUse {
            notify_vault_unlocked(&path);
        }
        Ok(status)
    }

    /// `SealCredentialVaultKey`: the unlocked key of this kernel's Vault, sealed to `request.target_public_key`.
    pub(crate) fn seal_credential_vault_key(
        &self,
        request: SealCredentialVaultKeyRequest,
    ) -> Result<crate::secret::VaultKeySeed, DaemonError> {
        let (path, _) = self.chariox_vault_path()?;
        let kernel = self.owned.config_projection.snapshot();
        crate::secret::export_vault_key_seed(
            &path,
            &kernel.daemon_id,
            &kernel.relay_private_key,
            &request.target_kernel_id,
            &request.target_public_key,
        )
        .map_err(|error| {
            if crate::secret::is_chariox_vault_locked_error(&error) {
                vault_locked_error()
            } else {
                error
            }
        })
    }
}

//! The owner's session-less Vault unlock, and the one counted check every way
//! of presenting the Vault passphrase goes through.
use super::runtime_vault_unlock_state::{expand_vault_path, vault_unlock_request_lock};
use super::vault_unlock_requests::notify_vault_unlocked;
use super::*;
use crate::config::{CredentialVaultBackend, CredentialVaultUnlockPolicy};

// Everything the owner reads about an unlock that did not happen lives here.
mod wording {
    pub(super) const WRONG_PASSPHRASE: &str = "That is not the Vault passphrase. Try again.";
    pub(super) const MISSING: &str =
        "This kernel has no Vault yet. Create it on that machine first.";
    pub(super) const LOCKED: &str = "The Vault is locked. Unlock it, then try again.";

    pub(super) fn rate_limited(seconds: u64) -> String {
        format!("Too many wrong passphrases. Try again in {seconds} seconds.")
    }
}

const WRONG_PASSPHRASE: &str = "vault_passphrase_wrong";
const RATE_LIMITED: &str = "vault_unlock_rate_limited";
const MISSING: &str = "vault_missing";
const LOCKED: &str = "vault_locked";

fn refusal(code: &'static str, message: impl Into<String>, retryable: bool) -> DaemonError {
    DaemonError::VaultUnlock {
        code,
        message: message.into(),
        retryable,
    }
}

/// A session-less operation that needs the Vault while it is locked: the
/// client unlocks it, then repeats the same operation.
pub(crate) fn vault_locked_error() -> DaemonError {
    refusal(LOCKED, wording::LOCKED, true)
}

/// Where the owner's own client leaves the Vault open afterwards: the policy's
/// lease, and under "ask every time" just the next operation.
fn owner_client_lease(
    config: &crate::config::UserCredentialVaultConfig,
) -> crate::secret::VaultUnlockLease {
    match config.unlock_policy {
        CredentialVaultUnlockPolicy::Always => crate::secret::VaultUnlockLease::SingleUse,
        CredentialVaultUnlockPolicy::KernelInit => crate::secret::VaultUnlockLease::KernelShutdown,
        CredentialVaultUnlockPolicy::Ttl => crate::secret::VaultUnlockLease::TtlMinutes(
            config
                .default_ttl_minutes
                .min(config.max_ttl_minutes)
                .max(1),
        ),
    }
}

impl KernelRuntimeState {
    /// `UnlockCredentialVault`: unlocks the existing Vault for the owner's
    /// own client. It never creates a Vault.
    pub(crate) async fn unlock_credential_vault_for_owner(
        &self,
        owner: &str,
        request: crate::local::UnlockCredentialVaultRequest,
    ) -> Result<(crate::secret::CharioxVaultUnlockStatus, &'static str), DaemonError> {
        let user_config = self.owned.config_projection.snapshot().user_config;
        let config = user_config.credential_vault;
        let path = expand_vault_path(&config.path);
        if config.backend != CredentialVaultBackend::CharioxEncrypted {
            return Ok((
                crate::secret::chariox_encrypted_vault_status(&path)?,
                "not_required",
            ));
        }
        let _mutation = vault_unlock_request_lock(&path).lock_owned().await;
        let status = crate::secret::chariox_encrypted_vault_status(&path)?;
        if status.unlocked {
            return Ok((status, "already_unlocked"));
        }
        let lease = owner_client_lease(&config);
        let status = self
            .unlock_vault_counted(
                owner,
                &path,
                request.passphrase.expose_secret(),
                lease,
                false,
            )
            .await?;
        if lease != crate::secret::VaultUnlockLease::SingleUse {
            notify_vault_unlocked(&path);
        }
        Ok((status, "unlocked"))
    }

    /// Checks `passphrase` against the Vault at `path`, one check at a time
    /// and inside the owner's shared wrong-answer budget (the passkey is this
    /// same passphrase). `create` allows the first-time interaction to make
    /// the Vault; the owner's own request never does.
    pub(super) async fn unlock_vault_counted(
        &self,
        owner: &str,
        path: &Path,
        passphrase: &str,
        lease: crate::secret::VaultUnlockLease,
        create: bool,
    ) -> Result<crate::secret::CharioxVaultUnlockStatus, DaemonError> {
        let attempts = &self.owned.critical_approval_passkeys;
        let _turn = attempts.verifying.clone().lock_owned().await;
        if let Some(remaining) = attempts.locked_for(owner, Instant::now()) {
            log_outcome(owner, path, "rate_limited");
            return Err(refusal(
                RATE_LIMITED,
                wording::rate_limited(remaining.as_secs().max(1)),
                true,
            ));
        }
        if !create && !path.exists() {
            log_outcome(owner, path, "missing");
            return Err(refusal(MISSING, wording::MISSING, false));
        }
        let (check_path, secret) = (
            path.to_owned(),
            zeroize::Zeroizing::new(passphrase.to_owned()),
        );
        let unlocked = tokio::task::spawn_blocking(move || {
            if create && !check_path.exists() {
                return crate::secret::unlock_chariox_encrypted_vault(&check_path, &secret, lease)
                    .map(Some);
            }
            crate::secret::unlock_existing_chariox_encrypted_vault(&check_path, &secret, lease)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "unlock vault",
            message: format!("Vault passphrase check stopped: {error}"),
        })??;
        let Some(status) = unlocked else {
            attempts.record_failure(owner, Instant::now());
            log_outcome(owner, path, "wrong");
            return Err(refusal(WRONG_PASSPHRASE, wording::WRONG_PASSPHRASE, true));
        };
        attempts.record_success(owner, Instant::now(), None);
        if let Err(error) = self.pin_critical_approval_verifier_after_unlock(path) {
            crate::secret::lock_chariox_encrypted_vault(path)?;
            crate::secret::clear_vault_secret_process_cache()?;
            return Err(error);
        }
        log_outcome(owner, path, "unlocked");
        Ok(status)
    }
}

fn log_outcome(owner: &str, path: &Path, outcome: &str) {
    crate::logging::info_with_fields(
        "credential_vault",
        "Chariox vault unlock attempt",
        serde_json::json!({
            "owner": owner,
            "path": path.display().to_string(),
            "outcome": outcome,
        }),
    );
}

//! Protocol 383: approving a critical action (a kernel decision whose choice is
//! marked `requires_passkey`) needs the Chariox passkey, which is the encrypted
//! vault's passphrase, or the owner's optional remember window. The vault is
//! usually unlocked, so its unlock proves no human is present; the passkey is
//! checked at answer time, then dropped. Deny and routine decisions never
//! reach this gate.
//!
//! The passkey is checked against a pinned commitment to the vault key
//! (`VaultPasskeyVerifier`), kept durably: pinned the first time the kernel
//! holds a proven key for the vault its boot configuration names (an unlock,
//! or the first right passkey). Changing the configured vault path or the file
//! later never moves it.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::KernelRuntimeState;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{ApprovalPasskey, PASSKEY_REMEMBER_MAX_MINUTES};
use crate::secret::VaultPasskeyVerifier;

pub(crate) const PASSKEY_REQUIRED: &str = "PASSKEY_REQUIRED";
pub(crate) const PASSKEY_REJECTED: &str = "PASSKEY_REJECTED";
pub(crate) const PASSKEY_RATE_LIMITED: &str = "PASSKEY_RATE_LIMITED";
pub(crate) const PASSKEY_UNAVAILABLE: &str = "PASSKEY_UNAVAILABLE";
const AUDIT_EVENT: &str = "critical_approval.passkey";
const PIN_EVENT: &str = "critical_approval.passkey_verifier";
const PIN_SUBJECT: &str = "chariox-vault";

/// Wrong passkeys allowed before each further one locks the owner out.
const FREE_FAILURES: u32 = 5;
const FIRST_LOCKOUT_SECS: u64 = 30;
const MAX_LOCKOUT_SECS: u64 = 15 * 60;

pub(crate) fn passkey_error(code: &str, message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "critical approval",
        message: format!("{code}: {message}"),
    }
}

#[derive(Debug, Default)]
struct OwnerPresence {
    failures: u32,
    locked_until: Option<Instant>,
    remember_until: Option<Instant>,
}

/// Per-owner failure count, lockout and remember window, in kernel memory
/// only: a restart forgets every window.
#[derive(Clone)]
pub(super) struct CriticalApprovalPasskeys {
    owners: Arc<std::sync::Mutex<BTreeMap<String, OwnerPresence>>>,
    /// One verification at a time, so parallel guesses cannot outrun the limit.
    verifying: Arc<tokio::sync::Mutex<()>>,
    /// The encrypted vault named by the boot configuration; a pin is only
    /// ever taken from it.
    boot_vault: Option<PathBuf>,
    pinned: Arc<std::sync::Mutex<Option<VaultPasskeyVerifier>>>,
}

impl CriticalApprovalPasskeys {
    pub(super) fn new(boot_config: &crate::config::UserCredentialVaultConfig) -> Self {
        Self {
            owners: Default::default(),
            verifying: Default::default(),
            boot_vault: (boot_config.backend
                == crate::config::CredentialVaultBackend::CharioxEncrypted)
                .then(|| super::runtime_vault_unlock_state::expand_vault_path(&boot_config.path)),
            pinned: Default::default(),
        }
    }

    /// The pinned verifier, from memory or durable state.
    fn pinned(
        &self,
        durable: &DurableKernelStateStore,
    ) -> Result<Option<VaultPasskeyVerifier>, DaemonError> {
        let mut pinned = self.pinned.lock().expect("passkey verifier poisoned");
        if pinned.is_none() {
            *pinned = durable
                .load_subject_events_by_kind(PIN_SUBJECT, PIN_EVENT, 1)?
                .into_iter()
                .next()
                .and_then(|event| serde_json::from_value(event.payload).ok());
        }
        Ok(pinned.clone())
    }

    /// Pins `verifier` durably unless one is pinned already.
    fn pin(
        &self,
        durable: &DurableKernelStateStore,
        verifier: VaultPasskeyVerifier,
    ) -> Result<(), DaemonError> {
        if self.pinned(durable)?.is_some() {
            return Ok(());
        }
        let payload = serde_json::to_value(&verifier).map_err(|error| {
            passkey_error(PASSKEY_UNAVAILABLE, &format!("passkey pin: {error}"))
        })?;
        durable.append_event(PIN_EVENT, Some(PIN_SUBJECT.to_owned()), payload)?;
        *self.pinned.lock().expect("passkey verifier poisoned") = Some(verifier);
        Ok(())
    }

    fn with_owner<R>(&self, owner: &str, apply: impl FnOnce(&mut OwnerPresence) -> R) -> R {
        let mut owners = self
            .owners
            .lock()
            .expect("critical approval passkey state poisoned");
        apply(owners.entry(owner.to_owned()).or_default())
    }

    fn remembered(&self, owner: &str, now: Instant) -> bool {
        self.with_owner(owner, |presence| {
            presence.remember_until.is_some_and(|until| now < until)
        })
    }

    fn locked_for(&self, owner: &str, now: Instant) -> Option<Duration> {
        self.with_owner(owner, |presence| {
            presence
                .locked_until
                .and_then(|until| until.checked_duration_since(now))
                .filter(|remaining| !remaining.is_zero())
        })
    }

    fn record_failure(&self, owner: &str, now: Instant) {
        self.with_owner(owner, |presence| {
            presence.failures = presence.failures.saturating_add(1);
            if let Some(extra) = presence.failures.checked_sub(FREE_FAILURES) {
                let seconds = (FIRST_LOCKOUT_SECS << extra.min(10)).min(MAX_LOCKOUT_SECS);
                presence.locked_until = Some(now + Duration::from_secs(seconds));
            }
        });
    }

    fn record_success(&self, owner: &str, now: Instant, remember_minutes: Option<u32>) {
        self.with_owner(owner, |presence| {
            presence.failures = 0;
            presence.locked_until = None;
            if let Some(minutes) = remember_minutes {
                presence.remember_until = Some(now + Duration::from_secs(u64::from(minutes) * 60));
            }
        });
    }

    /// Moves the owner's window and lockout into the past.
    #[cfg(test)]
    pub(super) fn expire_for_test(&self, owner: &str) {
        let past = Instant::now() - Duration::from_millis(1);
        self.with_owner(owner, |presence| {
            presence.remember_until = presence.remember_until.map(|_| past);
            presence.locked_until = presence.locked_until.map(|_| past);
        });
    }
}

impl KernelRuntimeState {
    /// Whether this answer proves the owner's presence for a passkey-gated
    /// choice. `Ok(false)` when the choice needs no passkey or the caller does
    /// not own the decision (the answer path then refuses on its own terms).
    /// Every gated answer is audited, with its outcome only.
    pub(super) async fn authorize_critical_approval(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        caller_user_id: Option<&str>,
        passkey: Option<&ApprovalPasskey>,
        remember_minutes: Option<u32>,
    ) -> Result<bool, DaemonError> {
        let Some((owner, operation_id)) =
            self.owned
                .passkey_gate(session_id, interaction_id, choice_id, caller_user_id)
        else {
            return Ok(false);
        };
        if remember_minutes.is_some_and(|minutes| {
            minutes == 0 || minutes > PASSKEY_REMEMBER_MAX_MINUTES || passkey.is_none()
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "critical approval",
                message: "passkey_remember_minutes takes 1 to 15 minutes, with a passkey".into(),
            });
        }
        let presence = &self.owned.critical_approval_passkeys;
        let audit = |outcome: &str| {
            self.audit_critical_approval(
                &owner,
                &operation_id,
                interaction_id,
                outcome,
                remember_minutes,
            )
        };
        let Some(passkey) = passkey else {
            if presence.remembered(&owner, Instant::now()) {
                audit("remembered")?;
                return Ok(true);
            }
            audit("missing")?;
            return Err(passkey_error(
                PASSKEY_REQUIRED,
                "approving this critical action needs your Chariox passkey",
            ));
        };
        let _verifying = presence.verifying.lock().await;
        if let Some(remaining) = presence.locked_for(&owner, Instant::now()) {
            audit("rate_limited")?;
            return Err(passkey_error(
                PASSKEY_RATE_LIMITED,
                &format!(
                    "too many wrong passkeys; try again in {} seconds",
                    remaining.as_secs().max(1)
                ),
            ));
        }
        let durable = self.owned.durable_state_store.clone();
        let pinned = presence.pinned(&durable)?;
        let boot_vault = presence.boot_vault.clone();
        let secret = zeroize::Zeroizing::new(passkey.expose_secret().to_owned());
        // Verify against the pin; without one yet, pin from the boot vault
        // (the kernel's unlocked key, else this passkey if it opens the vault).
        let checked = tokio::task::spawn_blocking(move || {
            if let Some(verifier) = pinned {
                return verifier.verify(&secret).map(|ok| (ok, None));
            }
            let path = boot_vault
                .ok_or_else(|| passkey_error(PASSKEY_UNAVAILABLE, "no encrypted Chariox vault"))?;
            if let Some(verifier) = VaultPasskeyVerifier::from_unlocked(&path)? {
                return Ok((verifier.verify(&secret)?, Some(verifier)));
            }
            Ok(
                match VaultPasskeyVerifier::from_passphrase(&path, &secret)? {
                    Some(verifier) => (true, Some(verifier)),
                    None => (false, None),
                },
            )
        })
        .await
        .map_err(|_| passkey_error(PASSKEY_UNAVAILABLE, "passkey verification stopped"))?;
        let verified = checked.and_then(|(verified, new_pin)| {
            if let Some(verifier) = new_pin {
                presence.pin(&durable, verifier)?;
            }
            Ok(verified)
        });
        match verified {
            Ok(true) => {
                presence.record_success(&owner, Instant::now(), remember_minutes);
                audit("verified")?;
                Ok(true)
            }
            Ok(false) => {
                presence.record_failure(&owner, Instant::now());
                audit("rejected")?;
                Err(passkey_error(
                    PASSKEY_REJECTED,
                    "the passkey is not correct",
                ))
            }
            Err(_) => {
                audit("unavailable")?;
                Err(passkey_error(
                    PASSKEY_UNAVAILABLE,
                    "critical approvals need the encrypted Chariox vault, set up with its passphrase",
                ))
            }
        }
    }

    /// Pins the verifier as soon as the kernel unlocks its boot vault, so a
    /// later change of vault path or file cannot supply the first pin.
    pub(super) fn pin_critical_approval_verifier_after_unlock(&self, vault: &std::path::Path) {
        let presence = &self.owned.critical_approval_passkeys;
        if presence.boot_vault.as_deref() != Some(vault) {
            return;
        }
        let durable = &self.owned.durable_state_store;
        if let Ok(Some(verifier)) = VaultPasskeyVerifier::from_unlocked(vault) {
            let _ = presence.pin(durable, verifier);
        }
    }

    #[cfg(test)]
    pub(crate) fn expire_critical_approval_presence_for_test(&self, owner: &str) {
        self.owned.critical_approval_passkeys.expire_for_test(owner);
    }

    fn audit_critical_approval(
        &self,
        owner: &str,
        operation_id: &str,
        interaction_id: &str,
        outcome: &str,
        remember_minutes: Option<u32>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            AUDIT_EVENT,
            Some(operation_id.to_owned()),
            serde_json::json!({
                "owner": owner,
                "interaction_id": interaction_id,
                "outcome": outcome,
                "remember_minutes": remember_minutes,
            }),
        )?;
        Ok(())
    }
}

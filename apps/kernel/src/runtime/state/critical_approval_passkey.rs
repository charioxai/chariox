//! Protocol 383: approving a critical action (a kernel decision whose choice is
//! marked `requires_passkey`) needs the Chariox passkey, which is the encrypted
//! vault's passphrase, or the owner's optional remember window. The vault is
//! usually unlocked, so its unlock proves no human is present; the passkey is
//! checked against the vault file at answer time, then dropped. Deny and
//! routine decisions never reach this gate.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::local::{ApprovalPasskey, PASSKEY_REMEMBER_MAX_MINUTES};

pub(crate) const PASSKEY_REQUIRED: &str = "PASSKEY_REQUIRED";
pub(crate) const PASSKEY_REJECTED: &str = "PASSKEY_REJECTED";
pub(crate) const PASSKEY_RATE_LIMITED: &str = "PASSKEY_RATE_LIMITED";
pub(crate) const PASSKEY_UNAVAILABLE: &str = "PASSKEY_UNAVAILABLE";
const AUDIT_EVENT: &str = "critical_approval.passkey";

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
#[derive(Clone, Default)]
pub(super) struct CriticalApprovalPasskeys {
    owners: Arc<std::sync::Mutex<BTreeMap<String, OwnerPresence>>>,
    /// One verification at a time, so parallel guesses cannot outrun the limit.
    verifying: Arc<tokio::sync::Mutex<()>>,
}

impl CriticalApprovalPasskeys {
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
        let vault = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        if vault.backend != crate::config::CredentialVaultBackend::CharioxEncrypted {
            audit("unavailable")?;
            return Err(passkey_error(
                PASSKEY_UNAVAILABLE,
                "critical approvals need the encrypted Chariox vault, whose passphrase is the passkey",
            ));
        }
        let path = super::runtime_vault_unlock_state::expand_vault_path(&vault.path);
        let secret = zeroize::Zeroizing::new(passkey.expose_secret().to_owned());
        let verified = tokio::task::spawn_blocking(move || {
            crate::secret::verify_chariox_vault_passphrase(&path, secret.as_str())
        })
        .await
        .map_err(|_| passkey_error(PASSKEY_UNAVAILABLE, "passkey verification stopped"))?;
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
                    "the Chariox vault could not be read; set it up with its passphrase first",
                ))
            }
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

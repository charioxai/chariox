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
//! later never moves it; only a passphrase change of that vault does, after
//! the current passphrase verifies against the pin (`change_vault_passphrase`).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::KernelRuntimeState;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{ApprovalPasskey, PASSKEY_REMEMBER_MAX_MINUTES};
use crate::secret::{CharioxVaultUnlockStatus, VaultPasskeyVerifier};

pub(crate) const PASSKEY_REQUIRED: &str = "PASSKEY_REQUIRED";
pub(crate) const PASSKEY_REJECTED: &str = "PASSKEY_REJECTED";
pub(crate) const PASSKEY_RATE_LIMITED: &str = "PASSKEY_RATE_LIMITED";
pub(crate) const PASSKEY_UNAVAILABLE: &str = "PASSKEY_UNAVAILABLE";
const AUDIT_EVENT: &str = "critical_approval.passkey";
const PIN_EVENT: &str = "critical_approval.passkey_verifier";
/// A pin move recorded before the vault file is re-keyed (`PinMove`).
const PIN_MOVE_EVENT: &str = "critical_approval.passkey_verifier_move";
const ROTATION_EVENT: &str = "critical_approval.passkey_rotation";
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

/// A passphrase change of the boot vault moves the pin from `previous` to
/// `next`. This is recorded before the vault file is re-keyed and its outcome
/// after; if the kernel stops in between, `settle` decides it.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PinMove {
    previous: VaultPasskeyVerifier,
    next: VaultPasskeyVerifier,
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

    /// The pinned verifier, from memory or durable state. A pin move newer
    /// than the last pin is a passphrase change whose outcome was not
    /// recorded; it is settled first.
    fn pinned(
        &self,
        durable: &DurableKernelStateStore,
    ) -> Result<Option<VaultPasskeyVerifier>, DaemonError> {
        let mut pinned = self.pinned.lock().expect("passkey verifier poisoned");
        if pinned.is_none() {
            let latest = |kind: &str| {
                durable
                    .load_subject_events_by_kind(PIN_SUBJECT, kind, 1)
                    .map(|events| events.into_iter().next())
            };
            let pin = latest(PIN_EVENT)?;
            *pinned = match latest(PIN_MOVE_EVENT)? {
                Some(moved) if pin.as_ref().is_none_or(|pin| moved.sequence > pin.sequence) => {
                    Some(self.settle(durable, moved.payload)?)
                }
                _ => pin.and_then(|event| serde_json::from_value(event.payload).ok()),
            };
        }
        Ok(pinned.clone())
    }

    /// Decides a pin move whose outcome was not recorded: it happened only if
    /// the boot vault file carries the new verifier's salt, which only that
    /// passphrase change wrote. Both sides were set by someone holding the
    /// passkey, so the file can only choose between them. The outcome is
    /// recorded, so this runs once.
    fn settle(
        &self,
        durable: &DurableKernelStateStore,
        payload: serde_json::Value,
    ) -> Result<VaultPasskeyVerifier, DaemonError> {
        let moved: PinMove = serde_json::from_value(payload).map_err(|error| {
            passkey_error(PASSKEY_UNAVAILABLE, &format!("passkey pin move: {error}"))
        })?;
        let verifier = if self
            .boot_vault
            .as_deref()
            .is_some_and(|vault| moved.next.matches_vault_file(vault))
        {
            moved.next
        } else {
            moved.previous
        };
        append_pin_event(durable, PIN_EVENT, &verifier)?;
        Ok(verifier)
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
        append_pin_event(durable, PIN_EVENT, &verifier)?;
        *self.pinned.lock().expect("passkey verifier poisoned") = Some(verifier);
        Ok(())
    }

    /// Records that the pin moves to `next`, before the vault file is
    /// re-keyed for it.
    fn record_pin_move(
        &self,
        durable: &DurableKernelStateStore,
        previous: &VaultPasskeyVerifier,
        next: &VaultPasskeyVerifier,
    ) -> Result<(), DaemonError> {
        append_pin_event(
            durable,
            PIN_MOVE_EVENT,
            &PinMove {
                previous: previous.clone(),
                next: next.clone(),
            },
        )
    }

    /// The blocking part of `change_vault_passphrase`. For the boot vault
    /// (`passkey`) with a pin, the current passphrase must verify against the
    /// pin (`Ok(None)` otherwise), and the pin move is recorded before the
    /// vault file is re-keyed and its outcome after. If recording the outcome
    /// fails, the pin is dropped from memory, so the next check settles the
    /// recorded move from the vault file. Without a pin yet, the new key is
    /// pinned once the file is re-keyed, as an unlock would pin it.
    fn change_passphrase(
        &self,
        durable: &DurableKernelStateStore,
        vault: &Path,
        passkey: bool,
        current: &str,
        new: &str,
    ) -> Result<Option<CharioxVaultUnlockStatus>, DaemonError> {
        let previous = if passkey { self.pinned(durable)? } else { None };
        if let Some(previous) = &previous {
            if !previous.verify(current)? {
                return Ok(None);
            }
        }
        let mut next = None;
        let changed = crate::secret::change_chariox_encrypted_vault_passphrase(
            vault,
            current,
            new,
            |verifier| {
                if let Some(previous) = &previous {
                    self.record_pin_move(durable, previous, verifier)?;
                }
                next = Some(verifier.clone());
                Ok(())
            },
        );
        // Record the outcome as the vault file shows it, as `settle` does
        // after a crash: a failed write may still have replaced the file.
        let outcome = next.filter(|_| passkey).and_then(|next| {
            if next.matches_vault_file(vault) {
                Some(next)
            } else {
                previous.clone()
            }
        });
        if let Some(verifier) = outcome {
            let recorded = append_pin_event(durable, PIN_EVENT, &verifier).is_ok();
            *self.pinned.lock().expect("passkey verifier poisoned") = recorded.then_some(verifier);
        }
        match changed {
            Ok(None) if previous.is_some() => Err(DaemonError::LocalTransport {
                operation: "credential_vault",
                message: "the passkey is right, but the Chariox vault file does not open with it; the passphrase is unchanged".into(),
            }),
            changed => changed,
        }
    }

    /// Ends every owner's remember window.
    fn end_remember_windows(&self) {
        let mut owners = self
            .owners
            .lock()
            .expect("critical approval passkey state poisoned");
        for presence in owners.values_mut() {
            presence.remember_until = None;
        }
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

fn append_pin_event(
    durable: &DurableKernelStateStore,
    kind: &str,
    value: &impl serde::Serialize,
) -> Result<(), DaemonError> {
    let payload = serde_json::to_value(value)
        .map_err(|error| passkey_error(PASSKEY_UNAVAILABLE, &format!("passkey pin: {error}")))?;
    durable.append_event(kind, Some(PIN_SUBJECT.to_owned()), payload)?;
    Ok(())
}

impl KernelRuntimeState {
    /// Changes the configured vault's passphrase. For the vault the boot
    /// configuration names, whose passphrase is the passkey, this is the
    /// passkey rotation (docs/CHARIOX_KERNEL_ACCESS_PLAN.md, 4.4): one check
    /// at a time under the passkey limit, the current passphrase must verify
    /// against the pin, the pin moves with the vault file through a recorded
    /// move that a restart settles, and every remember window ends. Each
    /// attempt is audited by outcome only.
    pub(super) async fn change_vault_passphrase(
        &self,
        owner: &str,
        vault: &Path,
        current: zeroize::Zeroizing<String>,
        new: zeroize::Zeroizing<String>,
    ) -> Result<CharioxVaultUnlockStatus, DaemonError> {
        let presence = self.owned.critical_approval_passkeys.clone();
        let passkey = presence.boot_vault.as_deref() == Some(vault);
        let audit = |outcome: &str| {
            self.owned.durable_state_store.append_event(
                ROTATION_EVENT,
                Some(PIN_SUBJECT.to_owned()),
                serde_json::json!({ "owner": owner, "passkey": passkey, "outcome": outcome }),
            )
        };
        let _verifying = presence.verifying.lock().await;
        if let Some(remaining) = passkey
            .then(|| presence.locked_for(owner, Instant::now()))
            .flatten()
        {
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
        let task = presence.clone();
        let vault_path = vault.to_path_buf();
        let changed = tokio::task::spawn_blocking(move || {
            task.change_passphrase(&durable, &vault_path, passkey, &current, &new)
        })
        .await
        .map_err(|_| passkey_error(PASSKEY_UNAVAILABLE, "the passphrase change stopped"))?;
        match changed {
            Ok(Some(status)) => {
                if passkey {
                    presence.record_success(owner, Instant::now(), None);
                    presence.end_remember_windows();
                }
                audit("changed")?;
                Ok(status)
            }
            Ok(None) => {
                if passkey {
                    presence.record_failure(owner, Instant::now());
                }
                audit("rejected")?;
                Err(DaemonError::LocalTransport {
                    operation: "credential_vault",
                    message: "the current Chariox vault passphrase is incorrect; the passphrase is unchanged".into(),
                })
            }
            Err(error) => {
                audit("failed")?;
                Err(error)
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "mixedcase!@#";
    const NEW: &str = "Mixed Case!@# \u{c9}";

    struct Fixture {
        root: PathBuf,
        vault: PathBuf,
        config: crate::config::UserCredentialVaultConfig,
        durable: DurableKernelStateStore,
    }

    impl Fixture {
        /// The boot vault with `OLD`, its key pinned.
        fn pinned() -> Self {
            let root =
                std::env::temp_dir().join(format!("passkey-move-{:016x}", rand::random::<u64>()));
            let vault = root.join("vault.json");
            crate::secret::create_chariox_encrypted_vault_for_test(&vault, OLD).unwrap();
            let config = crate::config::UserCredentialVaultConfig {
                backend: crate::config::CredentialVaultBackend::CharioxEncrypted,
                path: vault.display().to_string(),
                ..Default::default()
            };
            let durable = DurableKernelStateStore::open(root.join("state.sqlite")).unwrap();
            let fixture = Self {
                root,
                vault,
                config,
                durable,
            };
            let pin = VaultPasskeyVerifier::from_passphrase(&fixture.vault, OLD)
                .unwrap()
                .unwrap();
            fixture.kernel().pin(&fixture.durable, pin).unwrap();
            fixture
        }

        /// The passkey state of a freshly started kernel: nothing in memory.
        fn kernel(&self) -> CriticalApprovalPasskeys {
            CriticalApprovalPasskeys::new(&self.config)
        }

        fn accepts(&self, passkey: &str) -> bool {
            self.kernel()
                .pinned(&self.durable)
                .unwrap()
                .unwrap()
                .verify(passkey)
                .unwrap()
        }

        fn vault_opens_with(&self, passphrase: &str) -> bool {
            VaultPasskeyVerifier::from_passphrase(&self.vault, passphrase)
                .unwrap()
                .is_some()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_rotation_moves_the_pin_with_the_vault_and_a_wrong_passkey_moves_nothing() {
        let f = Fixture::pinned();
        let kernel = f.kernel();
        // The current passphrase is checked against the pin, not the file.
        assert!(kernel
            .change_passphrase(&f.durable, &f.vault, true, NEW, NEW)
            .unwrap()
            .is_none());
        assert!(f.accepts(OLD) && f.vault_opens_with(OLD));
        kernel
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .unwrap()
            .unwrap();
        assert!(kernel
            .pinned(&f.durable)
            .unwrap()
            .unwrap()
            .verify(NEW)
            .unwrap());
        // After a restart too, and the old passphrase opens neither.
        assert!(f.accepts(NEW) && !f.accepts(OLD));
        assert!(f.vault_opens_with(NEW) && !f.vault_opens_with(OLD));
        // A later replacement of the vault file does not move the recorded pin.
        crate::secret::create_chariox_encrypted_vault_for_test(&f.vault, "planted").unwrap();
        assert!(f.accepts(NEW) && !f.accepts("planted"));
    }

    #[test]
    fn a_rotation_stopped_after_the_vault_write_settles_on_the_new_passphrase() {
        let f = Fixture::pinned();
        let kernel = f.kernel();
        let previous = kernel.pinned(&f.durable).unwrap().unwrap();
        // The rotation's own steps, stopped (a crash) after the vault file is
        // re-keyed and before the new pin is recorded.
        crate::secret::change_chariox_encrypted_vault_passphrase(&f.vault, OLD, NEW, |next| {
            kernel.record_pin_move(&f.durable, &previous, next)
        })
        .unwrap()
        .unwrap();
        // The restarted kernel settles the move from the vault file, once.
        assert!(f.accepts(NEW) && !f.accepts(OLD));
        assert!(f.vault_opens_with(NEW));
        crate::secret::create_chariox_encrypted_vault_for_test(&f.vault, "planted").unwrap();
        assert!(f.accepts(NEW), "the settled pin is recorded");
    }

    #[test]
    fn a_rotation_stopped_before_the_vault_write_keeps_the_old_passphrase() {
        let f = Fixture::pinned();
        let kernel = f.kernel();
        let previous = kernel.pinned(&f.durable).unwrap().unwrap();
        // Stopped after the pin move is recorded, before the file is re-keyed.
        assert!(crate::secret::change_chariox_encrypted_vault_passphrase(
            &f.vault,
            OLD,
            NEW,
            |next| {
                kernel.record_pin_move(&f.durable, &previous, next)?;
                Err(passkey_error(PASSKEY_UNAVAILABLE, "stopped"))
            },
        )
        .is_err());
        assert!(f.accepts(OLD) && !f.accepts(NEW));
        assert!(f.vault_opens_with(OLD) && !f.vault_opens_with(NEW));
        // The same change then goes through.
        kernel
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .unwrap()
            .unwrap();
        assert!(f.accepts(NEW) && f.vault_opens_with(NEW));
    }
}

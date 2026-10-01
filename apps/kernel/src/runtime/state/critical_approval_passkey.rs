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
//!
//! Protocol 393: a passkey from a connection class that may not submit one
//! (kernel agents, hosts, relay peers) is refused before verification, and
//! every audit event names the answering connection's class.
//!
//! Protocol 394: the decision is a passkey prompt on every terminal of its
//! owner (`passkey_prompts`). Passkey answers are checked one at a time and
//! each holds its turn until its answer is applied, so a second terminal's
//! passkey for a prompt the first just answered is not verified: it is told
//! the prompt was already answered.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::KernelRuntimeState;
use crate::durable_state::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{ApprovalPasskey, KernelConnectionClass, PASSKEY_REMEMBER_MAX_MINUTES};
use crate::secret::VaultPasskeyVerifier;

pub(crate) const PASSKEY_REQUIRED: &str = "PASSKEY_REQUIRED";
pub(crate) const PASSKEY_REJECTED: &str = "PASSKEY_REJECTED";
pub(crate) const PASSKEY_RATE_LIMITED: &str = "PASSKEY_RATE_LIMITED";
pub(crate) const PASSKEY_UNAVAILABLE: &str = "PASSKEY_UNAVAILABLE";
pub(crate) const PASSKEY_NOT_ACCEPTED: &str = "PASSKEY_NOT_ACCEPTED";
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

/// Whether an answer proved the owner's presence. A verified passkey keeps
/// the verification turn until the answer is applied.
pub(super) struct CriticalApprovalAuthorization {
    pub(super) verified: bool,
    _turn: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl CriticalApprovalAuthorization {
    fn without_passkey(verified: bool) -> Self {
        Self {
            verified,
            _turn: None,
        }
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
    /// The pin in memory comes from a re-key whose new file is in use but
    /// not yet durable; its move is still unsettled in durable state.
    unsynced: Arc<std::sync::atomic::AtomicBool>,
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
            unsynced: Default::default(),
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
    /// recorded, so this runs once, but a new file only once its rename is
    /// durable: until the vault directory syncs, it is used in this process
    /// and the move stays unsettled, as after an unsynced change.
    fn settle(
        &self,
        durable: &DurableKernelStateStore,
        payload: serde_json::Value,
    ) -> Result<VaultPasskeyVerifier, DaemonError> {
        let moved: PinMove = serde_json::from_value(payload).map_err(|error| {
            passkey_error(PASSKEY_UNAVAILABLE, &format!("passkey pin move: {error}"))
        })?;
        let landed = match self.boot_vault.as_deref() {
            // Unreadable: the outcome is unknown, so the move stays unsettled.
            Some(vault) => moved.next.matches_vault_file(vault).map_err(|error| {
                passkey_error(
                    PASSKEY_UNAVAILABLE,
                    &format!("a passphrase change cannot be settled yet: {error}"),
                )
            })?,
            None => false,
        };
        let Some(vault) = self.boot_vault.as_deref().filter(|_| landed) else {
            append_pin_event(durable, PIN_EVENT, &moved.previous)?;
            return Ok(moved.previous);
        };
        if crate::secret::sync_chariox_encrypted_vault(vault).is_ok() {
            append_pin_event(durable, PIN_EVENT, &moved.next)?;
        } else {
            self.unsynced
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(moved.next)
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
    /// vault file is re-keyed and its outcome after, once the new file is
    /// durable. If recording the outcome fails, the pin is dropped from
    /// memory, so the next check settles the recorded move from the vault
    /// file. A change whose new file is in use but not durable takes effect in
    /// this process only; the next change first makes it durable and records
    /// it, or refuses. Without a pin yet, the new key is pinned once the file
    /// is re-keyed, as an unlock would pin it. Once the boot vault is re-keyed,
    /// every remember window ends.
    fn change_passphrase(
        &self,
        durable: &DurableKernelStateStore,
        vault: &Path,
        passkey: bool,
        current: &str,
        new: &str,
    ) -> Result<Option<CharioxVaultUnlockStatus>, DaemonError> {
        let previous = if passkey { self.pinned(durable)? } else { None };
        if passkey && self.unsynced.load(std::sync::atomic::Ordering::SeqCst) {
            // An earlier change's new file is in use but not yet durable:
            // make it durable and record its outcome before another move.
            crate::secret::sync_chariox_encrypted_vault(vault).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "credential_vault",
                    message: format!(
                        "the previous Chariox vault passphrase change is not yet on disk, so the passphrase is unchanged: {error}"
                    ),
                }
            })?;
            if let Some(pin) = &previous {
                append_pin_event(durable, PIN_EVENT, pin)?;
            }
            self.unsynced
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
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
        // The vault file decides the outcome, as `settle` does after a crash:
        // a write can fail after the new file is in place (at the directory
        // sync). Such a change is used in this process and ends every
        // remember window, but it may not survive a crash, so its move stays
        // unrecorded and a restart settles it from the file that survived.
        let attempted = next.is_some();
        let landed = match (&changed, &next) {
            (Ok(Some(_)), _) => Ok(true),
            (Err(_), Some(next)) => next.matches_vault_file(vault),
            _ => Ok(false),
        };
        let Ok(landed) = landed else {
            // The file cannot be read, so the outcome is unknown: the move
            // stays unsettled for the next check, and remember windows end in
            // case it landed.
            if passkey {
                self.end_remember_windows();
                *self.pinned.lock().expect("passkey verifier poisoned") = None;
            }
            return changed;
        };
        let committed = next.filter(|_| landed);
        let undurable = committed.is_some() && !matches!(changed, Ok(Some(_)));
        let outcome = if !passkey {
            None
        } else if committed.is_some() {
            self.end_remember_windows();
            committed
        } else {
            previous.clone().filter(|_| attempted)
        };
        if undurable {
            self.unsynced
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if let Some(verifier) = outcome {
            let in_use = undurable || append_pin_event(durable, PIN_EVENT, &verifier).is_ok();
            *self.pinned.lock().expect("passkey verifier poisoned") = in_use.then_some(verifier);
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
    /// choice. Not verified when the choice needs no passkey or the caller
    /// does not own the decision (the answer path then refuses on its own
    /// terms). Every gated answer is audited, with its outcome and the
    /// connection's class only. A passkey from a class that may not submit
    /// one is refused first, without verification or a count against the
    /// owner's limit.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn authorize_critical_approval(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        caller_user_id: Option<&str>,
        passkey: Option<&ApprovalPasskey>,
        remember_minutes: Option<u32>,
        connection_class: Option<KernelConnectionClass>,
    ) -> Result<CriticalApprovalAuthorization, DaemonError> {
        if passkey.is_some() && connection_class.is_some_and(|class| !class.may_submit_passkey()) {
            return Err(passkey_error(
                PASSKEY_NOT_ACCEPTED,
                "only a Chariox terminal can submit the passkey",
            ));
        }
        let Some((owner, operation_id)) =
            self.owned
                .passkey_gate(session_id, interaction_id, choice_id, caller_user_id)
        else {
            return Ok(CriticalApprovalAuthorization::without_passkey(false));
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
                connection_class,
            )
        };
        let Some(passkey) = passkey else {
            if presence.remembered(&owner, Instant::now()) {
                audit("remembered")?;
                return Ok(CriticalApprovalAuthorization::without_passkey(true));
            }
            audit("missing")?;
            return Err(passkey_error(
                PASSKEY_REQUIRED,
                "approving this critical action needs your Chariox passkey",
            ));
        };
        let turn = presence.verifying.clone().lock_owned().await;
        // Another terminal may have answered while this one waited its turn.
        if self
            .owned
            .passkey_gate(session_id, interaction_id, choice_id, caller_user_id)
            .is_none()
        {
            return Err(self.owned.closed_interaction_error(
                session_id,
                interaction_id,
                DaemonError::LocalTransport {
                    operation: "resolve runtime interaction",
                    message: format!("interaction {interaction_id} was not pending"),
                },
            ));
        }
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
                Ok(CriticalApprovalAuthorization {
                    verified: true,
                    _turn: Some(turn),
                })
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

    /// The local owner's critical-action decision, as the App validation pump
    /// raises it, for transport tests. It stays pending while the returned
    /// guard (its responder) lives.
    #[cfg(test)]
    pub(crate) async fn raise_critical_approval_for_test(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Box<dyn std::any::Any + Send> {
        use crate::session::{RuntimeInteraction, RuntimeInteractionChoice};
        self.create_kernel_operation_interaction(
            session_id,
            crate::session::DEFAULT_LOCAL_USER_ID,
            RuntimeInteraction::for_kernel_operation(
                interaction_id,
                format!("validation:{interaction_id}"),
                "Approve App action",
                "An App asks to perform a protected action.",
                vec![
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                    RuntimeInteractionChoice::new("approve", "Approve", "allow", None)
                        .requiring_passkey(),
                ],
            ),
        )
        .await
        .map(|responder| Box::new(responder) as Box<dyn std::any::Any + Send>)
        .expect("critical approval should be raised")
    }

    fn audit_critical_approval(
        &self,
        owner: &str,
        operation_id: &str,
        interaction_id: &str,
        outcome: &str,
        remember_minutes: Option<u32>,
        connection_class: Option<KernelConnectionClass>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            AUDIT_EVENT,
            Some(operation_id.to_owned()),
            critical_approval_audit_payload(
                owner,
                interaction_id,
                outcome,
                remember_minutes,
                connection_class,
            ),
        )?;
        Ok(())
    }
}

/// The `critical_approval.passkey` event: the outcome and, since protocol
/// 393, the answering connection's class. Never the passkey.
pub(crate) fn critical_approval_audit_payload(
    owner: &str,
    interaction_id: &str,
    outcome: &str,
    remember_minutes: Option<u32>,
    connection_class: Option<KernelConnectionClass>,
) -> serde_json::Value {
    serde_json::json!({
        "owner": owner,
        "interaction_id": interaction_id,
        "outcome": outcome,
        "remember_minutes": remember_minutes,
        "connection_class": connection_class,
    })
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
    fn a_rotation_whose_write_fails_after_the_rename_moves_the_pin_in_process_only() {
        let f = Fixture::pinned();
        let kernel = f.kernel();
        let before = std::fs::read(&f.vault).unwrap();
        kernel.record_success("owner", Instant::now(), Some(5));
        crate::secret::fail_next_vault_dir_sync_for_test();
        let error = kernel
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .expect_err("the failed directory sync is reported");
        assert!(error.to_string().contains("was changed"), "{error}");
        // The new file is in use, so this kernel's pin follows it and the
        // remember windows end.
        assert!(kernel
            .pinned(&f.durable)
            .unwrap()
            .unwrap()
            .verify(NEW)
            .unwrap());
        assert!(f.vault_opens_with(NEW));
        assert!(!kernel.remembered("owner", Instant::now()));
        // The rename was not synced, so a crash may bring the old file back:
        // the move stays pending and a restart settles on the surviving file.
        std::fs::write(&f.vault, before).unwrap();
        assert!(f.accepts(OLD) && !f.accepts(NEW));
    }

    #[test]
    fn a_rotation_whose_unsynced_rename_survives_settles_on_the_new_passphrase() {
        let f = Fixture::pinned();
        crate::secret::fail_next_vault_dir_sync_for_test();
        assert!(f
            .kernel()
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .is_err());
        assert!(f.accepts(NEW) && !f.accepts(OLD));
    }

    #[test]
    fn a_restart_records_an_unsynced_rotation_only_once_its_rename_is_durable() {
        let f = Fixture::pinned();
        let before = std::fs::read(&f.vault).unwrap();
        crate::secret::fail_next_vault_dir_sync_for_test();
        assert!(f
            .kernel()
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .is_err());
        // A restart sees the new file but cannot sync its directory: it uses
        // the new pin and records nothing...
        crate::secret::fail_next_vault_dir_sync_for_test();
        assert!(f.accepts(NEW));
        // ...so a crash that then loses the rename still settles on the old
        // file.
        std::fs::write(&f.vault, &before).unwrap();
        assert!(f.accepts(OLD) && !f.accepts(NEW));
    }

    #[test]
    fn a_rotation_after_an_unsynced_one_first_makes_it_durable_or_refuses() {
        const THIRD: &str = "third passphrase";
        let f = Fixture::pinned();
        let kernel = f.kernel();
        let before = std::fs::read(&f.vault).unwrap();
        crate::secret::fail_next_vault_dir_sync_for_test();
        assert!(kernel
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .is_err());
        // The next change cannot make that rename durable: it is refused and
        // records nothing, so a crash that loses the rename still recovers.
        crate::secret::fail_next_vault_dir_sync_for_test();
        let refused = kernel
            .change_passphrase(&f.durable, &f.vault, true, NEW, THIRD)
            .expect_err("an undurable earlier change blocks the next one");
        assert!(refused.to_string().contains("not yet on disk"), "{refused}");
        assert!(f.vault_opens_with(NEW));
        let after = std::fs::read(&f.vault).unwrap();
        std::fs::write(&f.vault, &before).unwrap();
        assert!(f.accepts(OLD) && !f.accepts(NEW));
        std::fs::write(&f.vault, after).unwrap();

        // Once the directory syncs, the earlier change is recorded and the
        // next one goes through.
        let f = Fixture::pinned();
        let kernel = f.kernel();
        crate::secret::fail_next_vault_dir_sync_for_test();
        assert!(kernel
            .change_passphrase(&f.durable, &f.vault, true, OLD, NEW)
            .is_err());
        kernel
            .change_passphrase(&f.durable, &f.vault, true, NEW, THIRD)
            .unwrap()
            .unwrap();
        assert!(f.accepts(THIRD) && !f.accepts(NEW) && !f.accepts(OLD));
        assert!(f.vault_opens_with(THIRD));
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
    fn a_restart_that_cannot_read_the_vault_leaves_the_rotation_unsettled() {
        let f = Fixture::pinned();
        let kernel = f.kernel();
        let previous = kernel.pinned(&f.durable).unwrap().unwrap();
        // Stopped after the re-key, before the new pin is recorded.
        crate::secret::change_chariox_encrypted_vault_passphrase(&f.vault, OLD, NEW, |next| {
            kernel.record_pin_move(&f.durable, &previous, next)
        })
        .unwrap()
        .unwrap();
        // While the vault cannot be read, nothing is settled or accepted...
        let aside = f.vault.with_extension("aside");
        std::fs::rename(&f.vault, &aside).unwrap();
        let unavailable = f.kernel().pinned(&f.durable).err().unwrap().to_string();
        assert!(unavailable.contains("PASSKEY_UNAVAILABLE"), "{unavailable}");
        // ...and once it can, the move settles on the file that is there.
        std::fs::rename(&aside, &f.vault).unwrap();
        assert!(f.accepts(NEW) && !f.accepts(OLD));
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

//! MP-10 / MP-11: a Vault login travels with a new managed machine, but the owner unlocks the
//! Vault at preflight and the export runs later, after Cloud has made the machine. Under "ask
//! every time" that unlock belongs to one operation, so preflight hands it over here: one use,
//! held for a bounded time, consumed by the export. A hold that ran out (or was used) makes the
//! export ask again through the launching session's unlock card. Holds are kept per Vault path and
//! launching session, so two launches never take over each other's ask.
use super::runtime_vault_unlock_state::{expand_vault_path, VaultUnlockGuard};
use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::time::Instant;

/// How long preflight's unlock waits for the export. Machine creation takes minutes; a lapsed
/// hold is not an error, the export asks again.
const VAULT_CARRY_HOLD: Duration = Duration::from_secs(10 * 60);

struct Held {
    id: u64,
    guard: VaultUnlockGuard,
    deadline: Instant,
}

struct Entry {
    /// The launching command, kept so a later export can show the unlock card in its session.
    ask: crate::runtime::command::KernelCommand,
    held: Option<Held>,
    /// Orders launches, so an export that names no known session takes the latest one.
    launched: u64,
}

/// A Vault path and the session that launched the machine.
type EntryKey = (PathBuf, Option<String>);

fn entries() -> &'static Mutex<BTreeMap<EntryKey, Entry>> {
    static ENTRIES: OnceLock<Mutex<BTreeMap<EntryKey, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn next_launch() -> u64 {
    static LAUNCHES: AtomicU64 = AtomicU64::new(0);
    LAUNCHES.fetch_add(1, Ordering::Relaxed)
}

fn locked_entries() -> std::sync::MutexGuard<'static, BTreeMap<EntryKey, Entry>> {
    entries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl KernelRuntimeState {
    fn carry_vault_path(&self) -> Option<PathBuf> {
        let vault = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .credential_vault;
        (vault.backend == crate::config::CredentialVaultBackend::CharioxEncrypted)
            .then(|| expand_vault_path(&vault.path))
    }

    /// Keeps preflight's unlock for the export that follows.
    pub(crate) fn hold_vault_for_login_export(
        &self,
        guard: VaultUnlockGuard,
        command: &crate::runtime::command::KernelCommand,
    ) {
        self.hold_vault_until(guard, command, Instant::now() + VAULT_CARRY_HOLD);
    }

    fn hold_vault_until(
        &self,
        guard: VaultUnlockGuard,
        command: &crate::runtime::command::KernelCommand,
        deadline: Instant,
    ) {
        let Some(path) = self.carry_vault_path() else {
            return;
        };
        let id = rand::random::<u64>();
        let key = (path.clone(), command.session_id.clone());
        {
            let mut entries = locked_entries();
            let entry = entries.entry(key.clone()).or_insert_with(|| Entry {
                ask: command.clone(),
                held: None,
                launched: 0,
            });
            entry.ask = command.clone();
            entry.launched = next_launch();
            let live = entry
                .held
                .as_ref()
                .is_some_and(|held| held.deadline > Instant::now());
            if live && !guard.locks_on_drop() {
                // The Vault is open on the earlier hold; this caller added nothing to keep.
                return;
            }
            if guard.locks_on_drop() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let until_ms = crate::session::unix_epoch_ms()
                    .saturating_add(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX));
                let _ = crate::secret::hold_chariox_vault_unlock_until(&path, until_ms);
            }
            // The new unlock replaces the old one, which must not lock it on its way out.
            if let Some(previous) = entry.held.replace(Held {
                id,
                guard,
                deadline,
            }) {
                previous.guard.disarm();
            }
        }
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            let lapsed = {
                let mut entries = locked_entries();
                entries
                    .get_mut(&key)
                    .filter(|entry| entry.held.as_ref().is_some_and(|held| held.id == id))
                    .and_then(|entry| entry.held.take())
            };
            // Nobody used the unlock in time: dropping the guard locks the Vault again.
            drop(lapsed);
        });
    }

    /// The Vault, open for one export. Takes the unlock preflight held; when that lapsed or was
    /// used, asks again with the unlock card of the session that launched the machine. An export
    /// that names no launch of its own takes the latest one. Dropping the guard ends the use.
    pub(crate) async fn vault_for_login_export(
        &self,
        session_id: Option<&str>,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let Some(path) = self.carry_vault_path() else {
            return Ok(VaultUnlockGuard::not_required());
        };
        let (held, ask) = {
            let mut entries = locked_entries();
            let own = (path.clone(), session_id.map(str::to_owned));
            let key = if entries.contains_key(&own) {
                Some(own)
            } else {
                entries
                    .iter()
                    .filter(|((entry_path, _), _)| *entry_path == path)
                    .max_by_key(|(_, entry)| entry.launched)
                    .map(|(key, _)| key.clone())
            };
            match key.and_then(|key| entries.get_mut(&key)) {
                Some(entry) => (entry.held.take(), Some(entry.ask.clone())),
                None => (None, None),
            }
        };
        if let Some(held) = held {
            if held.deadline > Instant::now() && held.guard.still_unlocked() {
                return Ok(held.guard);
            }
            drop(held);
        }
        match ask {
            Some(command) => {
                self.ensure_vault_unlocked_for_command_context(
                    &command,
                    None,
                    None,
                    "provider_account_portability",
                )
                .await
            }
            None if crate::secret::chariox_encrypted_vault_status(&path)?.unlocked => {
                Ok(VaultUnlockGuard::not_required())
            }
            None => Err(super::vault_unlock_request::vault_locked_error()),
        }
    }
}

#[cfg(test)]
mod tests;

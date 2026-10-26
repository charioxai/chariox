//! MP-08 / MP-10 / MP-11: a human wait must be visible in the requesting
//! session. Only the short Vault mutation is serialized across sessions.
use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{watch, Mutex as AsyncMutex};

struct VaultRequests {
    mutation: Arc<AsyncMutex<()>>,
    sessions: Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>,
    unlocked: watch::Sender<()>,
}

fn requests(path: &Path) -> Arc<VaultRequests> {
    static REQUESTS: OnceLock<Mutex<BTreeMap<PathBuf, Arc<VaultRequests>>>> = OnceLock::new();
    let path = super::runtime_vault_unlock_state::expand_vault_path(&path.to_string_lossy());
    REQUESTS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .expect("vault request map poisoned")
        .entry(path)
        .or_insert_with(|| {
            Arc::new(VaultRequests {
                mutation: Arc::new(AsyncMutex::new(())),
                sessions: Mutex::new(BTreeMap::new()),
                unlocked: watch::channel(()).0,
            })
        })
        .clone()
}

pub(super) fn vault_unlock_request_lock(path: &Path) -> Arc<AsyncMutex<()>> {
    requests(path).mutation.clone()
}

pub(super) fn vault_unlock_session_request_lock(path: &Path, session: &str) -> Arc<AsyncMutex<()>> {
    let requests = requests(path);
    let mut sessions = requests
        .sessions
        .lock()
        .expect("vault session requests poisoned");
    sessions.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = sessions.get(session).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    sessions.insert(session.to_owned(), Arc::downgrade(&lock));
    lock
}

pub(super) fn vault_unlock_notifications(path: &Path) -> watch::Receiver<()> {
    requests(path).unlocked.subscribe()
}

pub(super) fn notify_vault_unlocked(path: &Path) {
    requests(path).unlocked.send_replace(());
}

impl KernelRuntimeState {
    /// A successful unlock in another session resumes this wait through the
    /// same authority path and withdraws only this wait's obsolete prompt.
    pub(super) async fn await_vault_unlock_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        operation: &'static str,
        vault_path: &Path,
        unlocks: &mut watch::Receiver<()>,
        accept_shared_unlock: bool,
    ) -> Result<Option<PendingInteractionResolution>, DaemonError> {
        let id = interaction.id().to_owned();
        let wait = self.await_vault_interaction(session_id, interaction, operation, "vault unlock");
        tokio::pin!(wait);
        loop {
            tokio::select! {
                result = &mut wait => return result.map(Some),
                Ok(()) = unlocks.changed(), if accept_shared_unlock => {
                    if crate::secret::chariox_encrypted_vault_status(vault_path)?.unlocked {
                        self.timeout_runtime_interaction(session_id, &id).await?;
                        return Ok(None);
                    }
                }
            }
        }
    }
}

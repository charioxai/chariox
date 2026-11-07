//! MP-08 / MP-10 / MP-11: a human wait must be visible in the requesting
//! session. Only the short Vault mutation is serialized across sessions.
use super::runtime_vault_unlock_state::CloseInteractionOnDrop;
use super::*;
use crate::local::VaultUnlockSurface;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{watch, Mutex as AsyncMutex};

struct VaultRequests {
    mutation: Arc<AsyncMutex<()>>,
    sessions: Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>,
    unlocked: watch::Sender<Option<VaultUnlockSurface>>,
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
                unlocked: watch::channel(None).0,
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

pub(super) fn vault_unlock_notifications(
    path: &Path,
) -> watch::Receiver<Option<VaultUnlockSurface>> {
    requests(path).unlocked.subscribe()
}

pub(super) fn notify_vault_unlocked(path: &Path, surface: VaultUnlockSurface) {
    requests(path).unlocked.send_replace(Some(surface));
}

impl KernelRuntimeState {
    /// A successful unlock in another session resumes this wait through the
    /// same authority path and withdraws only this wait's obsolete prompt.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn await_vault_unlock_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        operation: &'static str,
        vault_path: &Path,
        unlocks: &mut watch::Receiver<Option<VaultUnlockSurface>>,
        deadline: tokio::time::Instant,
        card: &mut Option<CloseInteractionOnDrop>,
    ) -> Result<Option<PendingInteractionResolution>, DaemonError> {
        let id = interaction.id().to_owned();
        let agent_id = interaction.agent_id().map(str::to_owned);
        let waiting = agent_id
            .as_deref()
            .map(|agent| self.begin_vault_wait(session_id, agent))
            .unwrap_or_default();
        // The first attempt opens the card; a retry puts its refusal into the open one.
        let receiver = match card {
            None => {
                let receiver =
                    self.create_terminal_credential_interaction(session_id, interaction)?;
                *card = Some(CloseInteractionOnDrop {
                    state: self.clone(),
                    session_id: session_id.to_string(),
                    interaction_id: id.clone(),
                });
                receiver
            }
            Some(_) => self.owned.rearm_vault_card(session_id, interaction)?,
        };
        let wait = self.wait_for_vault_resolution(
            session_id,
            &id,
            receiver,
            operation,
            "vault unlock",
            Some(deadline),
        );
        tokio::pin!(wait);
        let result = loop {
            tokio::select! {
                biased;
                result = &mut wait => break result.map(Some),
                Ok(()) = unlocks.changed() => {
                    let surface = (*unlocks.borrow_and_update()).unwrap_or(VaultUnlockSurface::Card);
                    match crate::secret::chariox_encrypted_vault_status(vault_path) {
                        Ok(status) if status.unlocked => {
                            break self
                                .owned
                                .settle_vault_unlocked(session_id, &id, surface)
                                .map(|()| None);
                        }
                        Ok(_) => {}
                        Err(error) => break Err(error),
                    }
                }
            }
        };
        let refusal = match &result {
            Ok(Some(resolution)) if resolution.status == "timed_out" => {
                Some("The Vault unlock timed out".to_owned())
            }
            Ok(Some(resolution)) if !matches!(resolution.choice_id.as_deref(), Some(choice) if choice != "cancel") => {
                Some("The Vault unlock was declined".to_owned())
            }
            Err(error) => Some(format!("The Vault unlock failed: {error}")),
            _ => None,
        };
        // Boxed: callers hold this future across their own awaits and must stay small (MD-4).
        Box::pin(self.end_vault_wait(session_id, waiting, refusal)).await;
        result
    }
}

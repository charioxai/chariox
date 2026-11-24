//! MP-08 / MP-10 / MP-11: a human wait must be visible in the requesting
//! session. Only the short Vault mutation is serialized across sessions.
use super::runtime_vault_unlock_state::CloseInteractionOnDrop;
use super::*;
use crate::local::VaultUnlockSurface;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{watch, Mutex as AsyncMutex};

struct VaultRequests {
    mutation: Arc<AsyncMutex<()>>,
    /// Each Room's open unlock card, which every Vault use in that Room joins.
    rooms: Mutex<BTreeMap<String, watch::Receiver<Option<RoomUnlockEnd>>>>,
    unlocked: watch::Sender<Option<VaultUnlockSurface>>,
}

/// How a Room's unlock card ended, for every use that waited on it.
#[derive(Clone)]
pub(super) enum RoomUnlockEnd {
    /// Unlocked, naming the window when a person unlocked outside the card.
    Unlocked(Option<VaultUnlockSurface>),
    Refused(Arc<DaemonError>),
}

/// A Vault use's place at its Room's unlock card.
pub(super) enum RoomUnlock {
    /// A card is already open in the Room: wait for its end.
    Joined(watch::Receiver<Option<RoomUnlockEnd>>),
    /// No card is open: this use opens it and reports its end through the sender.
    Opened(
        watch::Sender<Option<RoomUnlockEnd>>,
        watch::Receiver<Option<RoomUnlockEnd>>,
    ),
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
                rooms: Mutex::new(BTreeMap::new()),
                unlocked: watch::channel(None).0,
            })
        })
        .clone()
}

pub(super) fn vault_unlock_request_lock(path: &Path) -> Arc<AsyncMutex<()>> {
    requests(path).mutation.clone()
}

/// Joins the Room's open unlock card, or opens it: one card per Room and Vault at a time.
pub(super) fn room_unlock(path: &Path, session: &str) -> RoomUnlock {
    let requests = requests(path);
    let mut rooms = requests.rooms.lock().expect("vault room requests poisoned");
    rooms.retain(|_, end| end.borrow().is_none() && end.has_changed().is_ok());
    if let Some(end) = rooms.get(session) {
        return RoomUnlock::Joined(end.clone());
    }
    let (sender, end) = watch::channel(None);
    rooms.insert(session.to_owned(), end.clone());
    RoomUnlock::Opened(sender, end)
}

pub(super) fn vault_unlock_notifications(
    path: &Path,
) -> watch::Receiver<Option<VaultUnlockSurface>> {
    requests(path).unlocked.subscribe()
}

/// How a wait for a Vault unlock ended.
pub(super) enum VaultUnlockWait {
    Answered(PendingInteractionResolution),
    /// A person unlocked the Vault in another window; the card is already withdrawn.
    UnlockedElsewhere(VaultUnlockSurface),
}

pub(super) fn notify_vault_unlocked(path: &Path, surface: VaultUnlockSurface) {
    requests(path).unlocked.send_replace(Some(surface));
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
        unlocks: &mut watch::Receiver<Option<VaultUnlockSurface>>,
        card: &mut Option<CloseInteractionOnDrop>,
    ) -> Result<VaultUnlockWait, DaemonError> {
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
            None,
        );
        tokio::pin!(wait);
        let result = loop {
            tokio::select! {
                biased;
                result = &mut wait => break result.map(VaultUnlockWait::Answered),
                Ok(()) = unlocks.changed() => {
                    let surface = (*unlocks.borrow_and_update()).unwrap_or(VaultUnlockSurface::Card);
                    match super::runtime_vault_unlock_state::vault_open_to_every_use(vault_path) {
                        Ok(true) => {
                            break self
                                .owned
                                .settle_vault_unlocked(session_id, &id, surface)
                                .map(|()| VaultUnlockWait::UnlockedElsewhere(surface));
                        }
                        Ok(false) => {}
                        Err(error) => break Err(error),
                    }
                }
            }
        };
        let refusal = match &result {
            Ok(VaultUnlockWait::Answered(resolution)) if resolution.status == "timed_out" => {
                Some("The Vault unlock card closed without an answer".to_owned())
            }
            Ok(VaultUnlockWait::Answered(resolution)) if !matches!(resolution.choice_id.as_deref(), Some(choice) if choice != "cancel") => {
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

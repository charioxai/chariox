use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock, Weak};

use tokio::sync::oneshot;
mod pollers;
use pollers::PendingPollers;

#[derive(Debug, Clone)]
pub(super) struct PendingMcpContinuation {
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) source_attachment_id: String,
    pub(super) mcp_name: String,
    pub(super) previous_prompt: String,
    pub(super) reload_reason: super::ProviderReloadReason,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingMcpContinuationStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingMcpContinuation>>>,
    pub(super) pollers: PendingPollers,
}

impl PendingMcpContinuationStore {
    pub(super) fn shared() -> Self {
        static STORE: OnceLock<PendingMcpContinuationStore> = OnceLock::new();
        STORE
            .get_or_init(PendingMcpContinuationStore::default)
            .clone()
    }

    pub(super) fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingMcpContinuation>> {
        self.inner
            .lock()
            .expect("pending MCP continuation mutex poisoned")
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingProviderReload {
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) reason: super::ProviderReloadReason,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingProviderReloadStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingProviderReload>>>,
    pub(super) pollers: PendingPollers,
}

impl PendingProviderReloadStore {
    pub(super) fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingProviderReload>> {
        self.inner
            .lock()
            .expect("pending provider reload mutex poisoned")
    }
}

#[derive(Debug, Clone)]
pub(super) struct PendingInteraction {
    pub(super) session_id: String,
    pub(super) session_store_identity: Weak<()>,
    pub(super) kernel_operation_owner: Option<String>,
    pub(super) kernel_operation_deadline: Option<std::time::Instant>,
    pub(super) responder: Arc<StdMutex<Option<oneshot::Sender<PendingInteractionResolution>>>>,
}

impl PendingInteraction {
    pub(super) fn belongs_to(&self, sessions: &crate::session::SessionStateStore) -> bool {
        sessions.matches_identity(&self.session_store_identity)
    }
}

#[derive(Clone)]
pub(in crate::runtime) struct PendingInteractionResolution {
    pub(crate) status: &'static str,
    pub(crate) choice_id: Option<String>,
    pub(crate) reply: Option<String>,
}

impl std::fmt::Debug for PendingInteractionResolution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingInteractionResolution")
            .field("status", &self.status)
            .field("choice_id", &self.choice_id)
            .field("reply", &self.reply.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingInteractionStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingInteraction>>>,
    pub(super) mutation: Arc<StdMutex<()>>,
}

impl PendingInteractionStore {
    pub(super) fn shared() -> Self {
        static STORE: OnceLock<PendingInteractionStore> = OnceLock::new();
        STORE.get_or_init(PendingInteractionStore::default).clone()
    }

    pub(super) fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingInteraction>> {
        self.inner
            .lock()
            .expect("pending interaction mutex poisoned")
    }

    /// Called while the shared mutation guard is held. A dead weak identity
    /// cannot be revived, so dropping these senders cannot consume a decision
    /// from a live kernel. Ordinary agent interactions keep their old lifetime.
    pub(super) fn prune_abandoned_kernel_owners(&self) {
        let mut pending = self.write();
        let abandoned = pending
            .iter()
            .filter(|(_, entry)| {
                entry.kernel_operation_owner.is_some()
                    && entry.session_store_identity.strong_count() == 0
            })
            .take(32)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in abandoned {
            pending.remove(&id);
        }
    }
}

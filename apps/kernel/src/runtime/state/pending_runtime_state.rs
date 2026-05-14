use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock};

use tokio::sync::oneshot;

#[derive(Debug, Clone)]
pub(super) struct PendingMcpContinuation {
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) source_attachment_id: String,
    pub(super) mcp_name: String,
    pub(super) previous_prompt: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingMcpContinuationStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingMcpContinuation>>>,
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
    pub(super) reason: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingProviderReloadStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingProviderReload>>>,
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
    pub(super) responder: Arc<StdMutex<Option<oneshot::Sender<PendingInteractionResolution>>>>,
}

#[derive(Debug, Clone)]
pub(in crate::runtime) struct PendingInteractionResolution {
    pub(crate) status: &'static str,
    pub(crate) choice_id: Option<String>,
    pub(crate) reply: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingInteractionStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingInteraction>>>,
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
}

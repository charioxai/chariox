use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, OnceLock, Weak};

use tokio::sync::oneshot;
mod pollers;
use pollers::PendingPollers;

#[derive(Debug, Clone)]
pub(super) struct PendingMcpContinuation {
    pub(super) session_id: String,
    pub(super) agent_id: String,

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
    pub(super) provisional_meta_activation: bool,
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
    /// Kernel-wide decisions live on the same interaction board without a session.
    pub(super) kernel_wide_interaction: Option<crate::session::RuntimeInteraction>,
    pub(super) session_id: String,
    /// Detached kernel decision; no Session projection exists for it.
    pub(super) user_domain_interaction: Option<crate::session::RuntimeInteraction>,
    pub(super) session_store_identity: Weak<()>,
    pub(super) agent_lifetime: Option<PendingAgentInteractionLifetime>,
    pub(super) kernel_operation_owner: Option<String>,
    /// Credential prompts retain their agent subject for terminal rendering,
    /// but their answers belong exclusively to the human owner.
    pub(super) terminal_credential_owner: Option<String>,
    pub(super) kernel_operation_deadline: Option<std::time::Instant>,
    /// Protocol 403: the popup projected to the owner's terminals, for a
    /// decision that needs the passkey.
    pub(super) passkey_prompt: Option<Arc<crate::local::PasskeyPrompt>>,
    pub(super) responder: Arc<StdMutex<Option<oneshot::Sender<PendingInteractionResolution>>>>,
}

/// Internal ownership of the originating prompt/run, including forwarded worker runs.
#[derive(Debug, Clone)]
pub(super) struct PendingAgentInteractionLifetime {
    pub(super) agent_id: String,
    pub(super) prompt_id: Option<String>,
    pub(super) native_turn_id: Option<String>,
    pub(super) worker: Option<PendingWorkerInteractionLifetime>,
    pub(super) provider_run_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct PendingWorkerInteractionLifetime {
    pub(super) leased_agent_id: String,
    pub(super) execution_lease_id: String,
    pub(super) provider_run_id: String,
    pub(super) binding_observed: Arc<std::sync::atomic::AtomicBool>,
}

impl PendingInteraction {
    pub(super) fn belongs_to(&self, sessions: &crate::session::SessionStateStore) -> bool {
        sessions.matches_identity(&self.session_store_identity)
    }

    /// The component that asked is gone or the interaction was answered: no
    /// receiver waits for a resolution any more.
    pub(super) fn nobody_waits(&self) -> bool {
        self.responder.lock().map_or(true, |responder| {
            responder.as_ref().is_none_or(|sender| sender.is_closed())
        })
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
#[allow(
    clippy::type_complexity,
    reason = "Keep the explicit PendingInteractionStore state or return type at the existing boundary"
)]
pub(super) struct PendingInteractionStore {
    pub(super) inner: Arc<StdMutex<BTreeMap<String, PendingInteraction>>>,
    pub(super) mutation: Arc<StdMutex<()>>,
    /// Last orphaned-decision pass (unix ms) per session-store identity, so
    /// kernels sharing this process-wide store never throttle each other.
    pub(super) orphan_sweeps: Arc<StdMutex<Vec<(std::sync::Weak<()>, u64)>>>,
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
                (entry.kernel_operation_owner.is_some()
                    || entry.terminal_credential_owner.is_some())
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

//! MP-08 / MP-10 / MP-11: reconnects subscribe to one home-owned decision.
use super::*;
use crate::provider::ProviderNativeInteractionResolution;
use crate::session::RuntimeInteraction;
use crate::transport::relay_peer::RemoteNativeInteractionContext;
use tokio::sync::watch;

// Receipts are ephemeral, bounded, and never written to history or logs.
const RECEIPT_RETENTION: Duration = Duration::from_secs(300);
const MAX_RECEIPTS: usize = 1024;
type Resolution = Result<ProviderNativeInteractionResolution, String>;

#[derive(Clone)]
struct ForwardedInteraction {
    context: RemoteNativeInteractionContext,
    interaction: RuntimeInteraction,
    execution_lease_id: String,
    receiver: watch::Receiver<Option<Resolution>>,
    task: tokio::task::AbortHandle,
}

#[derive(Clone, Default)]
pub(super) struct ForwardedNativeInteractions {
    entries: Arc<std::sync::Mutex<BTreeMap<(String, String), ForwardedInteraction>>>,
}

impl ForwardedNativeInteractions {
    /// A provider login reuses one interaction id for each phase. The worker
    /// retires a decided phase here, so the next phase is a new decision and
    /// only transport retries of the current one are replayed.
    pub(super) fn retire(&self, session_id: &str, interaction_id: &str) {
        let key = (session_id.to_owned(), interaction_id.to_owned());
        if let Some(entry) = self.entries.lock().ok().and_then(|mut e| e.remove(&key)) {
            entry.task.abort();
        }
    }
}

fn recovery_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "relay_forward_native_interaction",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    pub(in crate::runtime) async fn resumable_forwarded_native_interaction(
        &self,
        context: RemoteNativeInteractionContext,
        interaction: RuntimeInteraction,
    ) -> Result<ProviderNativeInteractionResolution, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        // MP-08 / MP-10 / MP-11: delayed requests can outlive their worker
        // binding. Preserve the normal stale-interaction timeout; the relay
        // router still authorizes every live worker before entering this path.
        let Some(lease) = agent.remote_execution() else {
            return Ok(ProviderNativeInteractionResolution {
                status: "timed_out".into(),
                choice_id: None,
                reply: None,
            });
        };
        let key = (context.home_session_id.clone(), interaction.id().to_owned());
        let mut receiver = {
            let mut entries = self
                .owned
                .forwarded_native_interactions
                .entries
                .lock()
                .map_err(|_| recovery_error("interaction recovery store is unavailable"))?;
            if let Some(existing) = entries.get(&key) {
                if existing.context != context
                    || existing.interaction != interaction
                    || existing.execution_lease_id != lease.execution_lease_id
                {
                    return Err(recovery_error(
                        "interaction replay does not match its original request",
                    ));
                }
                existing.receiver.clone()
            } else {
                if entries.len() >= MAX_RECEIPTS {
                    return Err(recovery_error("interaction recovery capacity reached"));
                }
                let (sender, receiver) = watch::channel(None);
                let retention = Duration::from_secs(interaction.timeout_sec().unwrap_or(300))
                    .saturating_add(crate::runtime::native_interaction_bridge::REMOTE_NATIVE_INTERACTION_RESPONSE_BUFFER)
                    .max(RECEIPT_RETENTION);
                let state = self.clone();
                let store = self.owned.forwarded_native_interactions.clone();
                let (task_context, task_interaction, task_key) =
                    (context.clone(), interaction.clone(), key.clone());
                // Transport cancellation must not cancel the permission. Its
                // original timeout and turn/lease lifetime still close it.
                let task = tokio::spawn(async move {
                    let result = crate::runtime::native_interaction_bridge::request_runtime_interaction_with_timeout(
                        &state, &task_context.home_session_id, task_interaction,
                        "relay_forward_native_interaction", Some(&task_context),
                    ).await.map_err(|error| error.to_string());
                    let _ = sender.send_replace(Some(result));
                    drop(state);
                    // Keep an answer given while disconnected available to a
                    // returning worker, then erase it even if no new calls arrive.
                    tokio::time::sleep(retention).await;
                    if let Ok(mut entries) = store.entries.lock() {
                        entries.remove(&task_key);
                    }
                });
                entries.insert(
                    key,
                    ForwardedInteraction {
                        context,
                        interaction,
                        execution_lease_id: lease.execution_lease_id.clone(),
                        receiver: receiver.clone(),
                        task: task.abort_handle(),
                    },
                );
                receiver
            }
        };
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result.map_err(|message| recovery_error(&message));
            }
            receiver
                .changed()
                .await
                .map_err(|_| recovery_error("interaction recovery stopped before resolution"))?;
        }
    }
}

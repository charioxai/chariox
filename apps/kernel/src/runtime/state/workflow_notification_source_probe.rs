//! Keeps the availability of remote notification sources current between inventory refreshes.
//! The Cloud inventory is read rarely and carries no per-kernel freshness, so a kernel that
//! hosts the source of a notification trigger is asked directly over the relay. Cloud is not
//! contacted.
use super::workflow_notification_peers::{accepted_source_list, VERSION};
use super::*;
use crate::durable_state::workflow_notifications::NotificationOperation;

/// How often each source kernel is asked.
const PROBE_INTERVAL_MS: u64 = 10_000;
/// A single lost answer changes nothing; the second consecutive one marks the kernel offline.
const MISSES_TO_OFFLINE: u32 = 2;

#[derive(Clone, Default)]
pub(super) struct NotificationSourceProbes {
    state: Arc<std::sync::Mutex<BTreeMap<(String, String), ProbeState>>>,
}

#[derive(Default)]
struct ProbeState {
    next_due_ms: u64,
    misses: u32,
    asking: bool,
}

impl NotificationSourceProbes {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<(String, String), ProbeState>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The kernels to ask now, each marked as being asked until its answer is recorded. A
    /// kernel no subscription follows any more is forgotten.
    fn claim_due(&self, wanted: &BTreeSet<(String, String)>, now: u64) -> Vec<(String, String)> {
        let mut state = self.lock();
        state.retain(|key, _| wanted.contains(key));
        wanted
            .iter()
            .filter(|key| {
                let probe = state.entry((*key).clone()).or_default();
                let due = !probe.asking && now >= probe.next_due_ms;
                if due {
                    probe.asking = true;
                    probe.next_due_ms = now.saturating_add(PROBE_INTERVAL_MS);
                }
                due
            })
            .cloned()
            .collect()
    }

    /// Records one answer; true when the kernel just became unreachable.
    fn record(&self, key: &(String, String), answered: bool) -> bool {
        let mut state = self.lock();
        let Some(probe) = state.get_mut(key) else {
            return false;
        };
        probe.asking = false;
        if answered {
            probe.misses = 0;
            return false;
        }
        probe.misses = probe.misses.saturating_add(1);
        probe.misses == MISSES_TO_OFFLINE
    }
}

impl KernelRuntimeState {
    /// Starts the probes that are due without holding the caller: a kernel that does not
    /// answer takes the request timeout.
    pub(super) fn schedule_notification_source_probes(&self, now: u64) {
        let due = self.claim_due_notification_source_probes(now);
        if due.is_empty() {
            return;
        }
        let runtime = self.clone();
        tokio::spawn(async move { runtime.ask_notification_source_kernels(due).await });
    }

    /// Asks every source kernel that is due and updates the triggers listening to it.
    #[cfg(test)]
    pub(super) async fn probe_notification_sources(&self, now: u64) {
        let due = self.claim_due_notification_source_probes(now);
        self.ask_notification_source_kernels(due).await;
    }

    pub(super) fn claim_due_notification_source_probes(&self, now: u64) -> Vec<(String, String)> {
        let home = self.owned.config_projection.snapshot().daemon_id;
        // The durable subscriptions keep the kernel of the source while the source is disabled
        // or missing from the kernel's list, so the kernel is still asked when it comes back.
        let wanted = self
            .agent_notification_subscriptions()
            .into_iter()
            .filter(|s| s.source_kernel_id != home)
            .map(|s| (s.owner_user_id, s.source_kernel_id))
            .collect();
        self.owned
            .notification_source_probes
            .claim_due(&wanted, now)
    }

    async fn ask_notification_source_kernels(&self, due: Vec<(String, String)>) {
        let mut asks = tokio::task::JoinSet::new();
        for key in due {
            let runtime = self.clone();
            asks.spawn(async move { runtime.ask_notification_source_kernel(key).await });
        }
        let mut changed = false;
        while let Some(done) = asks.join_next().await {
            changed |= done.unwrap_or(false);
        }
        if changed {
            let _ = self.sync_agent_notification_trigger_availability();
        }
    }

    /// True when the kernel's cached sources changed availability.
    async fn ask_notification_source_kernel(&self, key: (String, String)) -> bool {
        let (owner, kernel) = &key;
        let response = self
            .notification_peer(
                owner,
                kernel,
                RelayPeerRequest::ListWorkflowNotificationSources {
                    protocol_version: VERSION,
                },
            )
            .await;
        let fresh = match response {
            Ok(RelayPeerResponse::WorkflowNotificationSources {
                protocol_version: VERSION,
                sources,
            }) => accepted_source_list(kernel, sources),
            _ => None,
        };
        let offline = self
            .owned
            .notification_source_probes
            .record(&key, fresh.is_some());
        let Ok(cached) = self
            .owned
            .durable_state_store
            .notification_cached_sources(owner)
        else {
            return false;
        };
        let before = availability_of(&cached, kernel);
        let sources = match fresh {
            Some(sources) => sources,
            None if offline => cached
                .into_iter()
                .filter(|s| &s.kernel_id == kernel)
                .map(|mut s| {
                    s.available = false;
                    s
                })
                .collect(),
            None => return false,
        };
        let after = availability_of(&sources, kernel);
        self.owned
            .durable_state_store
            .notify(NotificationOperation::Cache {
                owner: owner.clone(),
                kernel: kernel.clone(),
                sources,
            })
            .is_ok()
            && before != after
    }
}

fn availability_of(
    sources: &[crate::local::WorkflowNotificationSourceSummary],
    kernel: &str,
) -> BTreeMap<String, bool> {
    sources
        .iter()
        .filter(|s| s.kernel_id == kernel)
        .map(|s| (s.source_id.clone(), s.available))
        .collect()
}

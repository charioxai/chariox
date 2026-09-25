use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Serialize remote lease updates with prompt admission, without blocking other leases.
#[derive(Clone, Default)]
pub(super) struct LeasedAgentOperations {
    lanes: Arc<Mutex<BTreeMap<String, Weak<AsyncMutex<()>>>>>,
    #[cfg(test)]
    acquire_probes: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::Notify>>>>,
}

impl LeasedAgentOperations {
    #[cfg(test)]
    pub(crate) fn notify_on_next_acquire_for_tests(
        &self,
        leased_agent_id: &str,
    ) -> Arc<tokio::sync::Notify> {
        let probe = Arc::new(tokio::sync::Notify::new());
        self.acquire_probes
            .lock()
            .expect("leased agent acquire probe map poisoned")
            .insert(leased_agent_id.to_string(), Arc::clone(&probe));
        probe
    }

    pub(super) async fn lock(&self, leased_agent_id: &str) -> OwnedMutexGuard<()> {
        let lane = {
            let mut lanes = self
                .lanes
                .lock()
                .expect("leased agent operation map poisoned");
            lanes.retain(|_, lane| lane.strong_count() > 0);
            if let Some(lane) = lanes.get(leased_agent_id).and_then(Weak::upgrade) {
                lane
            } else {
                let lane = Arc::new(AsyncMutex::new(()));
                lanes.insert(leased_agent_id.to_string(), Arc::downgrade(&lane));
                lane
            }
        };
        #[cfg(test)]
        if let Some(probe) = self
            .acquire_probes
            .lock()
            .expect("leased agent acquire probe map poisoned")
            .remove(leased_agent_id)
        {
            probe.notify_one();
        }
        lane.lock_owned().await
    }
}

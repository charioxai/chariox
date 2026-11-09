use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::runtime::projection::{ActorQueueSnapshot, ProviderRunActorHealthSnapshot};

#[cfg(debug_assertions)]
mod wait_trace;
#[cfg(all(test, debug_assertions))]
mod wait_trace_tests;

#[derive(Clone)]
pub(crate) struct ProviderRunOperationLanes {
    pub(super) lanes:
        Arc<crate::runtime::checked_lock::CheckedMutex<BTreeMap<String, Arc<Semaphore>>>>,
    health: Arc<ProviderRunActorHealthCounters>,
    #[cfg(debug_assertions)]
    wait_trace: Arc<wait_trace::LaneWaitTrace>,
    #[cfg(test)]
    acquire_probes: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::Notify>>>>,
}

impl Default for ProviderRunOperationLanes {
    fn default() -> Self {
        Self {
            lanes: Arc::new(crate::runtime::checked_lock::CheckedMutex::new(
                "provider_operation_lanes",
                BTreeMap::new(),
            )),
            health: Default::default(),
            #[cfg(debug_assertions)]
            wait_trace: Arc::new(wait_trace::LaneWaitTrace::from_env()),
            #[cfg(test)]
            acquire_probes: Default::default(),
        }
    }
}

pub(crate) struct ProviderRunOperationPermit {
    permit: Option<OwnedSemaphorePermit>,
    lanes: ProviderRunOperationLanes,
    provider_run_id: String,
}

#[derive(Debug, Default)]
struct ProviderRunActorHealthCounters {
    enqueued_commands: AtomicU64,
    enqueue_rejections: AtomicU64,
}

impl ProviderRunOperationLanes {
    #[cfg(all(test, debug_assertions))]
    fn with_wait_trace_for_tests() -> Self {
        Self {
            wait_trace: Arc::new(wait_trace::LaneWaitTrace::enabled_for_tests()),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn notify_on_next_acquire_for_tests(
        &self,
        provider_run_id: &str,
    ) -> Arc<tokio::sync::Notify> {
        let probe = Arc::new(tokio::sync::Notify::new());
        self.acquire_probes
            .lock()
            .expect("provider run acquire probe map poisoned")
            .insert(provider_run_id.to_string(), Arc::clone(&probe));
        probe
    }

    pub(crate) async fn acquire(&self, provider_run_id: &str) -> ProviderRunOperationPermit {
        let semaphore = {
            let mut lanes = self
                .lanes
                .lock()
                .expect("provider run operation lane map poisoned");
            Arc::clone(
                lanes
                    .entry(provider_run_id.to_string())
                    .or_insert_with(|| Arc::new(Semaphore::new(1))),
            )
        };
        #[cfg(test)]
        if let Some(probe) = self
            .acquire_probes
            .lock()
            .expect("provider run acquire probe map poisoned")
            .remove(provider_run_id)
        {
            probe.notify_one();
        }
        #[cfg(debug_assertions)]
        let permit = self.wait_trace.acquire(provider_run_id, semaphore).await;
        #[cfg(not(debug_assertions))]
        let permit = semaphore.acquire_owned().await;
        let permit = permit.expect("provider run operation lane semaphore closed");
        ProviderRunOperationPermit {
            permit: Some(permit),
            lanes: self.clone(),
            provider_run_id: provider_run_id.to_string(),
        }
    }

    pub(crate) fn try_acquire(&self, provider_run_id: &str) -> Option<ProviderRunOperationPermit> {
        let semaphore = {
            let mut lanes = self
                .lanes
                .lock()
                .expect("provider run operation lane map poisoned");
            Arc::clone(
                lanes
                    .entry(provider_run_id.to_string())
                    .or_insert_with(|| Arc::new(Semaphore::new(1))),
            )
        };
        let permit = semaphore.try_acquire_owned().ok()?;
        #[cfg(debug_assertions)]
        self.wait_trace.record_holder(provider_run_id);
        Some(ProviderRunOperationPermit {
            permit: Some(permit),
            lanes: self.clone(),
            provider_run_id: provider_run_id.to_string(),
        })
    }

    pub(super) fn forget(&self, provider_run_id: &str) {
        let mut lanes = self
            .lanes
            .lock()
            .expect("provider run operation lane map poisoned");
        let idle = lanes.get(provider_run_id).is_some_and(|semaphore| {
            semaphore.available_permits() == 1 && Arc::strong_count(semaphore) == 1
        });
        if idle {
            lanes.remove(provider_run_id);
        }
    }

    pub(crate) fn queue_snapshots(&self) -> Vec<ActorQueueSnapshot> {
        let lanes = self
            .lanes
            .lock()
            .expect("provider run operation lane map poisoned");
        let mut snapshots = lanes
            .iter()
            .map(|(provider_run_id, semaphore)| {
                ActorQueueSnapshot::new(
                    provider_run_id.clone(),
                    1,
                    usize::from(semaphore.available_permits() == 0),
                )
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.lane_id.cmp(&right.lane_id));
        snapshots
    }

    pub(crate) fn health_snapshot(&self) -> ProviderRunActorHealthSnapshot {
        ProviderRunActorHealthSnapshot {
            enqueued_commands: self.health.enqueued_commands.load(Ordering::Relaxed),
            enqueue_rejections: self.health.enqueue_rejections.load(Ordering::Relaxed),
        }
    }

    pub(super) fn record_command_enqueued(&self) {
        self.health
            .enqueued_commands
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_enqueue_rejection(&self) {
        self.health
            .enqueue_rejections
            .fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for ProviderRunOperationPermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        self.lanes.forget(&self.provider_run_id);
    }
}

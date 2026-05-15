use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::runtime::projection::{ActorQueueSnapshot, ProviderRunActorHealthSnapshot};

#[derive(Clone, Default)]
pub(crate) struct ProviderRunOperationLanes {
    pub(super) lanes: Arc<Mutex<BTreeMap<String, Arc<Semaphore>>>>,
    health: Arc<ProviderRunActorHealthCounters>,
}

#[derive(Debug, Default)]
struct ProviderRunActorHealthCounters {
    enqueued_commands: AtomicU64,
    enqueue_rejections: AtomicU64,
}

impl ProviderRunOperationLanes {
    pub(crate) async fn acquire(&self, provider_run_id: &str) -> OwnedSemaphorePermit {
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
        semaphore
            .acquire_owned()
            .await
            .expect("provider run operation lane semaphore closed")
    }

    pub(super) fn forget(&self, provider_run_id: &str) {
        let mut lanes = self
            .lanes
            .lock()
            .expect("provider run operation lane map poisoned");
        lanes.remove(provider_run_id);
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

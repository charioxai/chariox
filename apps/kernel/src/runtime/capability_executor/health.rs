//! Capability executor health counters and blocking-task admission.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;

pub(crate) const CAPABILITY_EXECUTOR_CONCURRENCY_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilityExecutorHealthSnapshot {
    pub max_concurrent_jobs: usize,
    pub available_permits: usize,
    pub submitted_jobs: u64,
    pub running_jobs: u64,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub rejected_jobs: u64,
    pub join_errors: u64,
}

#[derive(Clone)]
pub(crate) struct CapabilityExecutorHealthStore {
    permits: Arc<Semaphore>,
    max_concurrent_jobs: usize,
    submitted_jobs: Arc<AtomicU64>,
    running_jobs: Arc<AtomicU64>,
    completed_jobs: Arc<AtomicU64>,
    failed_jobs: Arc<AtomicU64>,
    rejected_jobs: Arc<AtomicU64>,
    join_errors: Arc<AtomicU64>,
}

impl CapabilityExecutorHealthStore {
    pub(crate) fn new(max_concurrent_jobs: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent_jobs)),
            max_concurrent_jobs,
            submitted_jobs: Arc::new(AtomicU64::new(0)),
            running_jobs: Arc::new(AtomicU64::new(0)),
            completed_jobs: Arc::new(AtomicU64::new(0)),
            failed_jobs: Arc::new(AtomicU64::new(0)),
            rejected_jobs: Arc::new(AtomicU64::new(0)),
            join_errors: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn snapshot(&self) -> CapabilityExecutorHealthSnapshot {
        CapabilityExecutorHealthSnapshot {
            max_concurrent_jobs: self.max_concurrent_jobs,
            available_permits: self.permits.available_permits(),
            submitted_jobs: self.submitted_jobs.load(Ordering::Relaxed),
            running_jobs: self.running_jobs.load(Ordering::Relaxed),
            completed_jobs: self.completed_jobs.load(Ordering::Relaxed),
            failed_jobs: self.failed_jobs.load(Ordering::Relaxed),
            rejected_jobs: self.rejected_jobs.load(Ordering::Relaxed),
            join_errors: self.join_errors.load(Ordering::Relaxed),
        }
    }
}

impl Default for CapabilityExecutorHealthStore {
    fn default() -> Self {
        Self::new(CAPABILITY_EXECUTOR_CONCURRENCY_LIMIT)
    }
}

pub(super) async fn spawn_capability<F>(
    operation: &'static str,
    health: CapabilityExecutorHealthStore,
    task: F,
) -> Result<LocalDaemonResponse, DaemonError>
where
    F: FnOnce() -> Result<LocalDaemonResponse, DaemonError> + Send + 'static,
{
    let Ok(permit) = Arc::clone(&health.permits).try_acquire_owned() else {
        health.rejected_jobs.fetch_add(1, Ordering::Relaxed);
        return Err(DaemonError::LocalTransport {
            operation,
            message: "capability executor is overloaded".to_string(),
        });
    };
    health.submitted_jobs.fetch_add(1, Ordering::Relaxed);
    health.running_jobs.fetch_add(1, Ordering::Relaxed);
    let joined = tokio::task::spawn_blocking(task).await;
    drop(permit);
    decrement_saturating(&health.running_jobs);
    match joined {
        Ok(Ok(response)) => {
            health.completed_jobs.fetch_add(1, Ordering::Relaxed);
            Ok(response)
        }
        Ok(Err(error)) => {
            health.failed_jobs.fetch_add(1, Ordering::Relaxed);
            Err(error)
        }
        Err(error) => {
            health.join_errors.fetch_add(1, Ordering::Relaxed);
            Err(DaemonError::LocalTransport {
                operation,
                message: error.to_string(),
            })
        }
    }
}

fn decrement_saturating(value: &AtomicU64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        current.checked_sub(1)
    });
}

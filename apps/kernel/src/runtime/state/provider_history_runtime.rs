//! MP-08 / MP-10: ordered, bounded handoff to the existing transcript writer.
use super::*;
use std::sync::Mutex as StdMutex;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

type PreparedHistory = Vec<super::prompt_transcript_owned_state::PreparedHistoryEntry>;
type HistoryJob = JoinHandle<Result<(), DaemonError>>;
type HistoryRun = Arc<tokio::sync::Mutex<Option<HistoryJob>>>;
const MAX_PENDING_HISTORY_BATCHES: usize = 64;

#[derive(Clone)]
pub(super) struct ProviderHistoryAppends {
    tails: Arc<StdMutex<BTreeMap<String, HistoryRun>>>,
    capacity: Arc<Semaphore>,
}
impl Default for ProviderHistoryAppends {
    fn default() -> Self {
        Self {
            tails: Arc::default(),
            capacity: Arc::new(Semaphore::new(MAX_PENDING_HISTORY_BATCHES)),
        }
    }
}
pub(super) struct ProviderHistoryLease {
    tail: tokio::sync::OwnedMutexGuard<Option<HistoryJob>>,
    permit: tokio::sync::OwnedSemaphorePermit,
}
impl ProviderHistoryAppends {
    async fn reserve(&self, run_id: &str) -> Result<ProviderHistoryLease, DaemonError> {
        let permit = self
            .capacity
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| history_task_error(e.to_string()))?;
        let gate = self
            .tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(run_id.into())
            .or_default()
            .clone();
        let tail = gate.lock_owned().await;
        Ok(ProviderHistoryLease { tail, permit })
    }
    fn enqueue(
        owned: KernelRuntimeOwnedState,
        entries: PreparedHistory,
        mut lease: ProviderHistoryLease,
    ) {
        let previous = lease.tail.take();
        let permit = lease.permit;
        let job = tokio::spawn(async move {
            let previous_error = match previous {
                Some(mut previous) => await_history_job(&mut previous).await.err(),
                None => None,
            };
            let current =
                tokio::task::spawn_blocking(move || owned.append_prepared_history_entries(entries))
                    .await
                    .map_err(|e| history_task_error(e.to_string()));
            drop(permit);
            current?;
            previous_error.map_or(Ok(()), Err)
        });
        *lease.tail = Some(job);
    }
    async fn flush_run(&self, run_id: &str) -> Result<(), DaemonError> {
        let gate = self
            .tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
            .cloned();
        if let Some(gate) = gate {
            // A producer cannot overtake this barrier while its tail is awaited.
            let mut tail = gate.lock().await;
            if let Some(job) = tail.as_mut() {
                // Keep the handle in the gate across await: cancellation of a
                // flush must not detach the write from its ordered successor.
                let result = await_history_job(job).await;
                tail.take();
                result?;
            }
        }
        Ok(())
    }
    pub(super) async fn flush_all(&self) -> Result<(), DaemonError> {
        let gates = self
            .tails
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut first_error = None;
        for gate in gates {
            let mut tail = gate.lock().await;
            if let Some(job) = tail.as_mut() {
                let result = await_history_job(job).await;
                tail.take();
                if let Err(e) = result {
                    first_error.get_or_insert(e);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
async fn await_history_job(job: &mut HistoryJob) -> Result<(), DaemonError> {
    job.await.map_err(|e| history_task_error(e.to_string()))?
}
fn history_task_error(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "append provider output history",
        message,
    }
}
impl KernelRuntimeState {
    pub(super) async fn reserve_provider_history(
        &self,
        run_id: &str,
        has_entries: bool,
    ) -> Result<Option<ProviderHistoryLease>, DaemonError> {
        if has_entries {
            self.provider_history_appends
                .reserve(run_id)
                .await
                .map(Some)
        } else {
            Ok(None)
        }
    }
    pub(super) fn enqueue_provider_history(
        &self,
        entries: PreparedHistory,
        lease: Option<ProviderHistoryLease>,
    ) {
        if let Some(lease) = lease {
            ProviderHistoryAppends::enqueue(self.owned.clone(), entries, lease);
        }
    }
    pub(crate) async fn flush_session_provider_history(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let runs = self
            .owned
            .provider_store
            .list_runs()
            .into_iter()
            .filter(|run| run.session_id() == session_id)
            .collect::<Vec<_>>();
        futures_util::future::try_join_all(
            runs.iter().map(|run| self.flush_provider_history(run.id())),
        )
        .await?;
        Ok(())
    }
    pub(super) async fn flush_provider_history(&self, run_id: &str) -> Result<(), DaemonError> {
        self.provider_history_appends.flush_run(run_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_history_flush_preserves_the_ordered_tail() {
        let appends = ProviderHistoryAppends::default();
        let release = Arc::new(tokio::sync::Notify::new());
        let ready = release.clone();
        let mut lease = appends.reserve("run").await.unwrap();
        *lease.tail = Some(tokio::spawn(async move {
            ready.notified().await;
            Ok(())
        }));
        drop(lease);
        let flushing = appends.clone();
        let flush = tokio::spawn(async move { flushing.flush_run("run").await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        flush.abort();
        assert!(flush.await.unwrap_err().is_cancelled());
        let lease = appends.reserve("run").await.unwrap();
        assert!(
            lease.tail.is_some(),
            "cancelled flush lost the pending write"
        );
        drop(lease);
        release.notify_one();
        appends.flush_run("run").await.unwrap();
    }

    #[tokio::test]
    async fn history_reservations_apply_bounded_backpressure() {
        let appends = ProviderHistoryAppends::default();
        let mut leases = Vec::new();
        for index in 0..MAX_PENDING_HISTORY_BATCHES {
            leases.push(appends.reserve(&format!("run-{index}")).await.unwrap());
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), appends.reserve("overflow"))
                .await
                .is_err()
        );
        leases.pop();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), appends.reserve("overflow"))
                .await
                .is_ok()
        );
    }
}

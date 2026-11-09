//! MP-08 / MP-09 / MP-10 / MP-11: the slow part of supervising one task runs apart from the
//! lifecycle sweep. A worker, a peer or a provider that does not answer holds only the work of the
//! task it belongs to; the sweep goes on with every other agent and starts the work again when it
//! finds it undone.
use super::*;
use std::collections::BTreeSet;
use std::future::Future;

/// How long one piece of work may run. The longest wait inside it is a relay request to a worker
/// (`relay_request_timeout_ms`, 60 s by default), and a cancellation walks a task and its children.
const WORK_LIMIT: Duration = Duration::from_secs(180);
/// How many pieces of work run at once; the sweep that finds more undone starts them later.
const WORK_AT_ONCE: usize = 32;

#[derive(Clone, Default)]
pub(super) struct TaskWorkFlight {
    running: Arc<std::sync::Mutex<BTreeSet<String>>>,
}

/// The right to run the work named `key`, held until the work ends, however it ends.
pub(super) struct TaskWorkClaim {
    running: Arc<std::sync::Mutex<BTreeSet<String>>>,
    key: String,
}

impl Drop for TaskWorkClaim {
    fn drop(&mut self) {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl TaskWorkFlight {
    /// `None` when the same work already runs or too much work runs.
    fn claim(&self, key: &str) -> Option<TaskWorkClaim> {
        let mut running = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if running.len() >= WORK_AT_ONCE || !running.insert(key.to_owned()) {
            return None;
        }
        Some(TaskWorkClaim {
            running: Arc::clone(&self.running),
            key: key.to_owned(),
        })
    }

    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

impl KernelRuntimeState {
    /// Runs `work` for the task named by `key` off the caller's path, at most once at a time and
    /// for at most `WORK_LIMIT`. Its failure stays supervised: the next sweep starts it again.
    pub(super) fn spawn_task_work<Work, Done>(&self, key: String, work: Work)
    where
        Work: FnOnce(KernelRuntimeState) -> Done + Send + 'static,
        Done: Future<Output = Result<(), DaemonError>> + Send + 'static,
    {
        let Some(claim) = self.task_work.claim(&key) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let _claim = claim;
            match tokio::time::timeout(WORK_LIMIT, work(state)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%key, error=%crate::secret_redaction::redact_secrets(&error.to_string()),
                        "MP-08/MP-09/MP-10/MP-11: task work retained for the next sweep");
                }
                Err(_) => {
                    tracing::warn!(%key, "MP-08/MP-09/MP-10/MP-11: task work ran out of time; the next sweep starts it again");
                }
            }
        });
    }

    /// Waits until no task work runs.
    #[cfg(test)]
    pub(crate) async fn task_work_settled(&self) {
        while !self.task_work.is_idle() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_work_does_not_start_twice_while_it_runs() {
        let flight = TaskWorkFlight::default();
        let first = flight.claim("cancel:task-1");
        assert!(first.is_some());
        assert!(flight.claim("cancel:task-1").is_none());
        assert!(
            flight.claim("goal-check:task-1").is_some(),
            "another kind of work, or another task, is not held by it"
        );
        drop(first);
        assert!(
            flight.claim("cancel:task-1").is_some(),
            "work that ended, however it ended, starts again"
        );
    }

    #[test]
    fn work_beyond_the_limit_waits_for_a_later_sweep() {
        let flight = TaskWorkFlight::default();
        let running = (0..WORK_AT_ONCE)
            .map(|i| flight.claim(&format!("cancel:task-{i}")).unwrap())
            .collect::<Vec<_>>();
        assert!(flight.claim("cancel:one-more").is_none());
        drop(running);
        assert!(flight.is_idle());
        assert!(flight.claim("cancel:one-more").is_some());
    }
}

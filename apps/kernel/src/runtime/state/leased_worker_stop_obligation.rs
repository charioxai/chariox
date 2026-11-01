//! MP-08 / MP-10 / MP-11 A10-S04: losing a worker's connection does not stop its
//! run. The home ends the turn and wakes its waiters, then keeps the worker's
//! run as an open obligation until the worker is told to stop it, or the
//! lease's absolute ceiling has passed.
use super::*;

/// A worker run the home ended its turn for without knowing the run stopped.
#[derive(Clone)]
pub(super) struct UnstoppedWorkerRun {
    binding: crate::agent::RemoteAgentBinding,
    home_prompt_id: String,
    worker_provider_run_id: String,
    opened_at_ms: u64,
}

/// (execution lease, worker provider run) of each open obligation.
pub(super) type UnstoppedWorkerRuns =
    Arc<std::sync::Mutex<BTreeMap<(String, String), UnstoppedWorkerRun>>>;

impl KernelRuntimeState {
    pub(super) fn open_worker_stop_obligation(
        &self,
        binding: &crate::agent::RemoteAgentBinding,
        home_prompt_id: &str,
        worker_provider_run_id: &str,
        now: u64,
    ) {
        self.owned
            .unstopped_worker_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                (
                    binding.execution_lease_id.clone(),
                    worker_provider_run_id.to_string(),
                ),
                UnstoppedWorkerRun {
                    binding: binding.clone(),
                    home_prompt_id: home_prompt_id.to_string(),
                    worker_provider_run_id: worker_provider_run_id.to_string(),
                    opened_at_ms: now,
                },
            );
    }

    /// Asks each worker to cancel the run the home gave up on. An obligation ends
    /// when the worker acknowledges the cancellation or no longer holds the run, or
    /// when the lease's absolute ceiling has passed since the home gave up. Any
    /// other answer, including none, keeps it for the next sweep.
    pub(super) async fn discharge_worker_stop_obligations(&self, now: u64) {
        let open: Vec<_> = self
            .owned
            .unstopped_worker_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(key, run)| (key.clone(), run.clone()))
            .collect();
        let discharged = futures_util::future::join_all(open.into_iter().map(|(key, run)| async move {
            let expired = now.saturating_sub(run.opened_at_ms)
                >= crate::app::REMOTE_EXECUTION_LEASE_MAX_LIFETIME_MS;
            let stopped = !expired
                && match self
                    .peer_request_to_worker(
                        run.binding.clone(),
                        RelayPeerRequest::CancelLeasedPrompt {
                            leased_agent_id: run.binding.leased_agent_id.clone(),
                            home_prompt_id: run.home_prompt_id.clone(),
                            worker_provider_run_id: run.worker_provider_run_id.clone(),
                        },
                    )
                    .await
                {
                    Ok(Ok(RelayPeerResponse::LeasedPromptCancelled { .. })) => true,
                    Ok(Err(error)) => super::remote_prompt_lifecycle_runtime::remote_prompt_cancellation_should_treat_as_settled(&error),
                    _ => false,
                };
            (stopped || expired).then_some(key)
        }))
        .await;
        let mut unstopped = self
            .owned
            .unstopped_worker_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for key in discharged.into_iter().flatten() {
            unstopped.remove(&key);
        }
    }
}

#[cfg(test)]
impl KernelRuntimeState {
    pub(crate) fn unstopped_worker_runs_for_test(&self) -> usize {
        self.owned
            .unstopped_worker_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

//! MP-08 / MP-10 / MP-11 A10-S04: losing a worker's connection does not stop its
//! run. The home ends the turn and wakes its waiters, then keeps the worker's
//! run as a stored obligation until the worker has been told to stop it. Nothing
//! the home can see proves a worker stopped on its own: only a worker enforces
//! its own lease ceiling, and an ordinary kernel enforces none, so neither
//! elapsed time nor the removal of the agent or its Room ends the obligation.
//! It belongs to the exact lease and run, not to the Room, and is swept without it.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, UnstoppedWorkerRun, WorkerStopMutation};

/// Longest wait between two attempts to reach a worker that does not answer.
const MAX_RETRY_DELAY_MS: u64 = 10 * 60 * 1_000;

impl KernelRuntimeState {
    /// Stores the obligation. It must be stored before the home settles the
    /// turn: once the turn is settled nothing else knows the worker's run.
    pub(super) fn open_worker_stop_obligation(
        &self,
        room_id: &str,
        agent_id: &str,
        binding: &crate::agent::RemoteAgentBinding,
        home_prompt_id: &str,
        worker_provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::WorkerStop(WorkerStopMutation::Open(
                UnstoppedWorkerRun {
                    binding: binding.clone(),
                    room_id: room_id.to_string(),
                    agent_id: agent_id.to_string(),
                    home_prompt_id: home_prompt_id.to_string(),
                    worker_provider_run_id: worker_provider_run_id.to_string(),
                    attempts: 0,
                    next_attempt_ms: 0,
                },
            )))
            .map(|_| ())
    }

    /// Asks each worker to cancel the run the home gave up on. An obligation ends
    /// only when the worker acknowledges the cancellation or answers that it no
    /// longer holds the run or its leased agent. Any other answer, including
    /// none, keeps it for a later sweep, spaced further apart after each failure.
    pub(super) async fn discharge_worker_stop_obligations(
        &self,
        now: u64,
    ) -> Result<(), DaemonError> {
        let due: Vec<_> = self
            .owned
            .durable_state_store
            .unstopped_worker_runs()?
            .into_iter()
            .filter(|run| run.next_attempt_ms <= now && self.home_turn_is_settled(run))
            .collect();
        let attempts = futures_util::future::join_all(due.into_iter().map(|run| async move {
            let stopped = match self
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
            (run, stopped)
        }))
        .await;
        for (run, stopped) in attempts {
            let mutation = if stopped {
                WorkerStopMutation::Discharge(run)
            } else {
                let delay = crate::durable_state::agent_lifecycle::SWEEP_MS
                    .saturating_mul(1 << run.attempts.min(10))
                    .min(MAX_RETRY_DELAY_MS);
                WorkerStopMutation::Retry {
                    run,
                    next_attempt_ms: now.saturating_add(delay),
                }
            };
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::WorkerStop(mutation))?;
        }
        Ok(())
    }

    /// The home turn the worker run belonged to is no longer active: the
    /// obligation is stored before the home settles it, and a crash in between
    /// leaves a turn that is still being recovered.
    fn home_turn_is_settled(&self, run: &UnstoppedWorkerRun) -> bool {
        let Ok(session) = self.owned.session_store.get_session(&run.room_id) else {
            return true;
        };
        self.owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &run.agent_id)
            .is_none_or(|prompt| prompt.id() != run.home_prompt_id)
    }
}

#[cfg(test)]
impl KernelRuntimeState {
    pub(crate) fn unstopped_worker_runs_for_test(&self) -> usize {
        self.owned
            .durable_state_store
            .unstopped_worker_runs()
            .unwrap()
            .len()
    }
}

//! A kernel start moves every armed deadline later by its downtime. The worker that runs a
//! leased agent kept counting meanwhile and enforces the value it holds, so the home asks it
//! to take the shifted one. Until it confirms, the worker's value governs: a worker that
//! refuses for good keeps it, and the home goes back to that value.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, WorkerHeldDeadline};
use crate::local::AgentDeadlineState;

/// A worker's answer that will not change on a retry: the worker (or the relay for it) said no.
fn refused_for_good(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::RelayTransport {
            retryable: false,
            ..
        }
    ) || matches!(error, DaemonError::MachineRequest { code, .. } if code == "agent_deadline_worker_refused")
}

impl KernelRuntimeState {
    /// Asks every worker at once, each bounded by one request timeout, so an unreachable worker
    /// delays no other. The deadline tick is never held across a request.
    pub(super) async fn rearm_leased_agent_deadlines(&self) -> Result<(), DaemonError> {
        let _rearm = self.owned.agent_deadline_rearm.lock().await;
        let pending = {
            let _deadline_edit = self.owned.agent_deadline_edit.lock().await;
            self.leased_deadlines_to_rearm()?
        };
        let answers =
            futures_util::future::join_all(pending.iter().map(|(_, agent, deadline_ms)| {
                self.arm_leased_agent_deadline(agent, Some(*deadline_ms))
            }))
            .await;
        let _deadline_edit = self.owned.agent_deadline_edit.lock().await;
        let store = &self.owned.durable_state_store;
        let mut first_error = None;
        for ((held, _, _), answer) in pending.into_iter().zip(answers) {
            let (room, agent) = (held.room_id.clone(), held.agent_id.clone());
            match answer {
                Ok(()) => {
                    store.agent_lifecycle(Operation::DeadlineRearmed { room, agent })?;
                }
                Err(error) if refused_for_good(&error) => {
                    store.agent_lifecycle(Operation::DeadlineKeptByWorker { room, agent })?;
                    self.announce_worker_kept_deadline(&held, &error);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// The entries a worker must still confirm, with the agent and the deadline to send. An entry
    /// for an agent that is not leased, or whose deadline is no longer armed, ends here.
    fn leased_deadlines_to_rearm(
        &self,
    ) -> Result<Vec<(WorkerHeldDeadline, crate::agent::AgentInstance, u64)>, DaemonError> {
        let store = &self.owned.durable_state_store;
        let mut pending = Vec::new();
        for held in store.agent_deadlines_awaiting_rearm()? {
            let armed = store
                .agent_deadline(&held.room_id, &held.agent_id)?
                .filter(|deadline| deadline.state == AgentDeadlineState::Armed);
            let leased = self
                .owned
                .agent_store
                .get_agent(&held.agent_id)
                .ok()
                .filter(|agent| agent.remote_execution().is_some());
            match (armed, leased) {
                (Some(deadline), Some(agent)) => pending.push((held, agent, deadline.deadline_ms)),
                _ => {
                    store.agent_lifecycle(Operation::DeadlineRearmed {
                        room: held.room_id,
                        agent: held.agent_id,
                    })?;
                }
            }
        }
        Ok(pending)
    }

    fn announce_worker_kept_deadline(&self, held: &WorkerHeldDeadline, error: &DaemonError) {
        tracing::warn!(%error, room_id=%held.room_id, agent_id=%held.agent_id,
            "the worker refused the deadline this kernel's restart moved; its own deadline governs");
        self.owned.record_notice_for_agent(
            &held.room_id,
            None,
            Some(&held.agent_id),
            self.owned
                .attachment_store
                .list_session_attachment_ids(&held.room_id),
            format!(
                "Agent deadline: the worker running this agent kept its deadline of {} instead of the later one this kernel's restart gave back ({error}). The worker's deadline governs",
                super::agent_wake_scheduler::iso(held.held_ms),
            ),
        );
    }
}

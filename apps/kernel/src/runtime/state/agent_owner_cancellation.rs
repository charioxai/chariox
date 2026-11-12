//! Exact owner controls for retained tasks and timers.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

impl KernelRuntimeState {
    fn ensure_owner_agent_cancellation(
        &self,
        command: &crate::runtime::command::KernelCommand,
        room: &str,
        attachment: &str,
    ) -> Result<String, DaemonError> {
        let session = self.owned.session_store.get_session(room)?;
        // The identity the room keeps for its owner: `local` in a room made before enrollment.
        let owner = self.room_member_user_id_in(
            session.owner_user_id(),
            &crate::runtime::command::command_caller_user_id(command),
        );
        let attachment = self.owned.ensure_attachment_in_session(room, attachment)?;
        if !command.is_terminal_caller()
            || session.owner_user_id() != owner
            || attachment.owner_user_id() != owner
            || !matches!(
                attachment.capability_level(),
                crate::attachment::ClientCapabilityLevel::FullTerminal
                    | crate::attachment::ClientCapabilityLevel::InteractiveStructured
            )
        {
            return Err(crate::durable_state::agent_lifecycle::error(
                "only the room owner terminal can cancel a task or timer",
            ));
        }
        Ok(owner)
    }

    pub(crate) async fn cancel_owner_agent_task(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CancelAgentTaskRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let owner = self.ensure_owner_agent_cancellation(
            command,
            &request.session_id,
            &request.attachment_id,
        )?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(&request.session_id), None)?
            .into_iter()
            .find(|task| task.task_id == request.task_id)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error("task unavailable in this room")
            })?;
        if task.state == crate::durable_state::agent_lifecycle::ExecutionState::Done {
            return Err(crate::durable_state::agent_lifecycle::error(
                "task already completed; reload its current state",
            ));
        }
        let Outcome::Task(cancelled) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::CancelTask {
                    task: task.task_id,
                    owner,
                    revision: request.revision,
                })?
        else {
            unreachable!()
        };
        self.retract_stale_task_owner_interactions(&request.session_id)
            .await;
        Box::pin(self.cancel_agent_task_resources(&cancelled)).await?;
        self.agent_cancellation_response(&request.session_id, &cancelled.task_id)
    }

    pub(crate) async fn cancel_owner_agent_wake(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CancelAgentWakeRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let owner = self.ensure_owner_agent_cancellation(
            command,
            &request.session_id,
            &request.attachment_id,
        )?;
        let Outcome::Wakes(wakes) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::CancelOwnerTimer {
                    id: request.wake_id,
                    room: request.session_id.clone(),
                    owner,
                })?
        else {
            unreachable!()
        };
        self.agent_cancellation_response(&request.session_id, &wakes[0].task_id)
    }

    /// The owner ends the wait of a task from the agent's pane: its sources stop, the wait row
    /// says cancelled, and the agent is not woken.
    pub(crate) async fn cancel_owner_agent_wait(
        &self,
        command: &crate::runtime::command::KernelCommand,
        request: crate::local::CancelAgentWaitRequest,
    ) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
        let owner = self.ensure_owner_agent_cancellation(
            command,
            &request.session_id,
            &request.attachment_id,
        )?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(&request.session_id), None)?
            .into_iter()
            .find(|task| task.task_id == request.task_id)
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error("task unavailable in this room")
            })?;
        let Outcome::Wakes(stopped) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::CancelOwnerWait {
                    task: task.task_id.clone(),
                    room: request.session_id.clone(),
                    owner,
                    started_at_ms: request.started_at_ms,
                })?
        else {
            unreachable!()
        };
        self.owned.record_wait_cancelled(&task);
        for wake in &stopped {
            self.stop_cancelled_wait_source(wake).await;
        }
        self.owned
            .session_store
            .clear_agent_unread_output(&task.room_id, &task.agent_id)?;
        self.agent_cancellation_response(&request.session_id, &task.task_id)
    }

    /// The ledger already settled the wake; this stops what still runs for it. A source that does
    /// not stop reports its end to a task that is no longer registered, so nobody is woken.
    async fn stop_cancelled_wait_source(
        &self,
        wake: &crate::durable_state::agent_lifecycle::AgentWake,
    ) {
        let stopped = if wake.kind == "process" {
            self.cancel_agent_watched_process(&wake.id).await.map(drop)
        } else if wake.kind == "timer" {
            // The ledger owns the clock; settling the wake stopped it.
            Ok(())
        } else {
            self.owned
                .agent_wakes
                .drivers
                .get(&wake.kind)
                .and_then(|driver| driver.cancel(wake))
                .map(drop)
        };
        if let Err(error) = stopped {
            tracing::warn!(%error, wake = %wake.id, "cancelled wait source not stopped");
        }
        self.close_cards_following(wake);
    }
}

#[cfg(test)]
#[path = "enrolled_owner_terminal_controls_tests.rs"]
mod enrolled_owner_terminal_controls_tests;

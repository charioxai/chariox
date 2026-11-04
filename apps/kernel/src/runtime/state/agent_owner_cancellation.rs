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
        let owner = crate::runtime::command::command_caller_user_id(command);
        let session = self.owned.session_store.get_session(room)?;
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
        self.retract_stale_task_owner_interactions(
            &request.session_id,
            std::slice::from_ref(&cancelled),
        )
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
}

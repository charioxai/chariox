//! Publish a completed task whether it ended at a provider boundary or while handling a receipt.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState, Operation};

impl KernelRuntimeState {
    pub(super) async fn publish_agent_task_completion(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&crate::provider::TurnOutcome>,
        public_answer: Option<serde_json::Value>,
    ) -> Result<(), DaemonError> {
        let other_unfinished = self
            .owned
            .durable_state_store
            .agent_has_other_unfinished_task(&task.room_id, &task.agent_id, &task.task_id)?;
        if !other_unfinished {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    now: crate::session::unix_epoch_ms(),
                    public_answer: public_answer.clone(),
                    room: task.room_id.clone(),
                    source: task.agent_id.clone(),
                    occurrence: format!("task-terminal-{}", task.task_id),
                    success: task.state == ExecutionState::Done,
                })?;
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::SourceOutcome {
                now: crate::session::unix_epoch_ms(),
                public_answer: public_answer.clone(),
                room: task.room_id.clone(),
                source: task.prompt_id.clone(),
                occurrence: format!("task-terminal-{}", task.task_id),
                success: task.state == ExecutionState::Done,
            })?;
        if matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
            && task.task_id != task.prompt_id
        {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    now: crate::session::unix_epoch_ms(),
                    public_answer: public_answer.clone(),
                    room: task.room_id.clone(),
                    source: task.task_id.clone(),
                    occurrence: format!("task-terminal-{}", task.task_id),
                    success: task.state == ExecutionState::Done,
                })?;
        }
        // The parent hears of every ended task, whatever else the child still has open.
        if let Err(error) = self
            .announce_child_finished(task, failure, public_answer.as_ref())
            .await
        {
            tracing::warn!(%error, agent_id=%task.agent_id,
                "child_finished retained for the inbox sweep");
        }
        self.spawn_peer_agent_outcome_delivery();
        Ok(())
    }
}

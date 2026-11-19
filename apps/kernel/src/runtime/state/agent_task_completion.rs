//! Publish a completed task whether it ended at a provider boundary or while handling a receipt.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState, Operation};

impl KernelRuntimeState {
    pub(super) async fn publish_agent_task_completion(
        &self,
        task: &AgentTaskExecution,
        failure: Option<&crate::provider::TurnOutcome>,
        mut public_answer: Option<serde_json::Value>,
    ) -> Result<(), DaemonError> {
        let interrupted = if failure.is_none()
            && task.state == ExecutionState::Done
            && self
                .owned
                .agent_store
                .get_agent(&task.agent_id)
                .is_ok_and(|agent| agent.parent_agent_id().is_some())
            && self
                .owned
                .operational_history_store
                .foreground_command_interrupted_by_restart(
                    &task.room_id,
                    &task.agent_id,
                    &task.prompt_id,
                )? {
            let outcome = crate::provider::TurnOutcome::interrupted_by_restart(
                crate::app::FOREGROUND_COMMAND_RESTART_REASON,
            );
            if let Some(answer) = public_answer.as_mut() {
                answer["outcome"] = serde_json::json!(&outcome);
            }
            Some(outcome)
        } else {
            None
        };
        let failure = failure.or(interrupted.as_ref());
        let outcome = self.owned.finished_task_outcome(task, failure);
        let success = matches!(
            outcome.kind,
            crate::provider::TurnOutcomeKind::Completed
                | crate::provider::TurnOutcomeKind::AnsweredInTurn
        );
        let source_answer = public_answer.clone().or_else(|| {
            outcome
                .answered_in_turn
                .is_some()
                .then(|| serde_json::json!({"outcome": outcome}))
        });
        let other_unfinished = self
            .owned
            .durable_state_store
            .agent_has_other_unfinished_task(&task.room_id, &task.agent_id, &task.task_id)?;
        if !other_unfinished {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    now: crate::session::unix_epoch_ms(),
                    public_answer: source_answer.clone(),
                    room: task.room_id.clone(),
                    source: task.agent_id.clone(),
                    occurrence: format!("task-terminal-{}", task.task_id),
                    success,
                })?;
        }
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::SourceOutcome {
                now: crate::session::unix_epoch_ms(),
                public_answer: source_answer.clone(),
                room: task.room_id.clone(),
                source: task.prompt_id.clone(),
                occurrence: format!("task-terminal-{}", task.task_id),
                success,
            })?;
        if matches!(task.state, ExecutionState::Done | ExecutionState::Cancelled)
            && task.task_id != task.prompt_id
        {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::SourceOutcome {
                    now: crate::session::unix_epoch_ms(),
                    public_answer: source_answer.clone(),
                    room: task.room_id.clone(),
                    source: task.task_id.clone(),
                    occurrence: format!("task-terminal-{}", task.task_id),
                    success,
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

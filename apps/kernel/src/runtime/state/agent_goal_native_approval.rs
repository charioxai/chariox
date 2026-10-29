//! MP-08/MP-09/MP-10/MP-11 A12-S04: native decisions retain their goal block revision.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome,
};

impl KernelRuntimeOwnedState {
    pub(super) fn begin_goal_native_approval(
        &self,
        session: &str,
        interaction: &crate::session::RuntimeInteraction,
        lifetime: Option<&PendingAgentInteractionLifetime>,
    ) -> Result<Option<PendingGoalApproval>, DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools
            || interaction.kind() != crate::session::RuntimeInteractionKind::Permission
        {
            return Ok(None);
        }
        let Some(lifetime) = lifetime else {
            return Ok(None);
        };
        let task = self
            .durable_state_store
            .agent_tasks(Some(session), Some(&lifetime.agent_id))?
            .into_iter()
            .find(|task| {
                task.goal.is_some()
                    && task.state == ExecutionState::Working
                    && lifetime.prompt_id.as_deref().is_none_or(|prompt| {
                        task.pending_prompt_id.as_deref().unwrap_or(&task.prompt_id) == prompt
                    })
            });
        let Some(task) = task else { return Ok(None) };
        let prompt = task
            .pending_prompt_id
            .clone()
            .unwrap_or_else(|| task.prompt_id.clone());
        let Outcome::Task(blocked) =
            self.durable_state_store
                .agent_lifecycle(Operation::BeginGoalApproval {
                    task: task.task_id,
                    prompt: prompt.clone(),
                    now: crate::session::unix_epoch_ms(),
                })?
        else {
            return Ok(None);
        };
        Ok(Some(PendingGoalApproval {
            task_id: blocked.task_id,
            prompt_id: prompt,
            blocked_revision: blocked.blocked_revision,
        }))
    }

    pub(super) fn answer_goal_native_approval(
        &self,
        approval: Option<&PendingGoalApproval>,
    ) -> Result<(), DaemonError> {
        if let Some(approval) = approval {
            self.durable_state_store
                .agent_lifecycle(Operation::AnswerGoalApproval {
                    task: approval.task_id.clone(),
                    prompt: approval.prompt_id.clone(),
                    revision: approval.blocked_revision,
                    now: crate::session::unix_epoch_ms(),
                })?;
        }
        Ok(())
    }

    pub(super) fn goal_has_native_approval(&self, task: &AgentTaskExecution) -> bool {
        if task.goal.is_none() {
            return false;
        }
        // Registration holds the session write guard until the decision and its goal binding exist.
        let Ok(_session) = self.session_store.get_session(&task.room_id) else {
            return false;
        };
        self.pending_interactions.write().values().any(|pending| {
            pending.belongs_to(&self.session_store)
                && pending.session_id == task.room_id
                && pending.goal_approval.as_ref().is_some_and(|approval| {
                    approval.task_id == task.task_id
                        && approval.blocked_revision == task.blocked_revision
                })
        })
    }
}

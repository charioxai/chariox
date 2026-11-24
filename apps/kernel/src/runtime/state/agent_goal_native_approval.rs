//! MP-08/MP-09/MP-10/MP-11 A12-S04: native decisions retain their goal block revision.
use super::*;
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, ExecutionState, Operation, Outcome,
};
use crate::provider::NativeApprovalGoal;

#[cfg(unix)]
impl KernelRuntimeState {
    pub(super) async fn resume_handed_over_goal_approval(
        &self,
        room: &str,
        agent: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Option<Result<(), DaemonError>> {
        let (run_id, approval) = crate::hot_reload::handed_over_goal_approval(room, agent)?;
        if approval.prompt_id != prompt.id() {
            return None;
        }
        match self.owned.durable_state_store.agent_task(&approval.task_id) {
            Ok(Some(task))
                if goal_approval_matches_turn(&task, room, agent, &run_id, &approval) => {}
            Ok(_) => return None,
            Err(error) => return Some(Err(error)),
        }
        Some(
            self.adopt_goal_approval_turn(room, agent, prompt, run_id, &approval)
                .await,
        )
    }

    async fn adopt_goal_approval_turn(
        &self,
        room: &str,
        agent: &str,
        prompt: &crate::session::PromptQueueItem,
        handed_over_run_id: String,
        approval: &NativeApprovalGoal,
    ) -> Result<(), DaemonError> {
        let run_id = self
            .recover_local_prompt_provider(room, agent, prompt)
            .await?;
        if run_id != handed_over_run_id {
            return Err(DaemonError::LocalTransport {
                operation: "restore goal approval",
                message: "the original Codex run was not adopted".into(),
            });
        }
        let _permit = self.provider_runtime_lanes.acquire(&run_id).await;
        let owned = &self.owned;
        let task = owned.durable_state_store.agent_task(&approval.task_id)?;
        if !task
            .is_some_and(|task| goal_approval_matches_turn(&task, room, agent, &run_id, approval))
        {
            return Err(DaemonError::LocalTransport {
                operation: "restore goal approval",
                message: "the goal is no longer blocked on this approval".into(),
            });
        }
        let run = owned.ensure_provider_run_in_session(room, &run_id)?;
        let dispatch =
            super::restart_recovery_runtime::restart_recovery_dispatch(room, agent, run_id, prompt);
        if !owned.ensure_prompt_dispatch_matches_active_prompt(&dispatch)? {
            return Ok(());
        }
        // The original turn is already bound to the task. New dispatch would reject a
        // blocked task or mutate its blocker. Submission on the adopted run reads its
        // retained Codex turn, which still waits for the original approval.
        owned.note_prompt_started(&dispatch.provider_run_id);
        self.submit_structured_prompt_with_handoff(
            &dispatch,
            &run,
            None,
            prompt.hidden_system_context(),
            super::structured_prompt_dispatch::SteerReceipt::Ignored,
        )
        .await?;
        Ok(())
    }
}

#[cfg(unix)]
fn goal_approval_matches_turn(
    task: &AgentTaskExecution,
    room: &str,
    agent: &str,
    run: &str,
    approval: &NativeApprovalGoal,
) -> bool {
    task.task_id == approval.task_id
        && task.room_id == room
        && task.agent_id == agent
        && task.state == ExecutionState::Blocked
        && task.blocked_revision == approval.blocked_revision
        && task.provider_run_id.as_deref() == Some(run)
        && task.pending_prompt_id.as_deref().unwrap_or(&task.prompt_id) == approval.prompt_id
        && task.goal.as_ref().is_some_and(|goal| !goal.paused)
}

impl KernelRuntimeOwnedState {
    pub(super) fn begin_goal_native_approval(
        &self,
        session: &str,
        interaction: &crate::session::RuntimeInteraction,
        lifetime: Option<&PendingAgentInteractionLifetime>,
    ) -> Result<Option<NativeApprovalGoal>, DaemonError> {
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
        Ok(Some(NativeApprovalGoal {
            task_id: blocked.task_id,
            prompt_id: prompt,
            blocked_revision: blocked.blocked_revision,
        }))
    }

    pub(super) fn answer_goal_native_approval(
        &self,
        approval: Option<&NativeApprovalGoal>,
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
        #[cfg(unix)]
        if let Some(run) = task.provider_run_id.as_deref() {
            if crate::hot_reload::retained_goal_approval(run).is_some_and(|approval| {
                goal_approval_matches_turn(task, &task.room_id, &task.agent_id, run, &approval)
            }) {
                return true;
            }
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

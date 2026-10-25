//! MP-08 / MP-09 / MP-10 / MP-11: restart retains the owner's blocked disposition.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};

impl KernelRuntimeState {
    pub(super) fn blocked_turn_has_executor(&self, task: &AgentTaskExecution) -> bool {
        let Ok(agent) = self.owned.agent_store.get_agent(&task.agent_id) else {
            return true;
        };
        agent.remote_execution().is_some()
            || self
                .owned
                .provider_store
                .get_run_for_agent(&task.room_id, &task.agent_id)
                .is_some()
    }

    pub(super) async fn restore_blocked_prompt_owner_decision(
        &self,
        room: &str,
        agent: &str,
        prompt_id: &str,
    ) -> Result<bool, DaemonError> {
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|task| task.prompt_id == prompt_id && task.state == ExecutionState::Blocked);
        let Some(task) = task else { return Ok(false) };
        self.ensure_task_owner_interaction(task).await?;
        Ok(true)
    }

    /// A blocked turn retains its prompt across restart, but has no provider executor.
    /// Only an accepted owner Resume may retire that old invocation and submit the
    /// durable continuation. Never advance queued work or replay the original input.
    pub(super) fn retire_blocked_prompt_without_executor(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(&task.agent_id)?;
        if agent.remote_execution().is_some()
            || self
                .owned
                .provider_store
                .get_run_for_agent(&task.room_id, &task.agent_id)
                .is_some()
        {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(&task.room_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &task.agent_id)
            .is_some_and(|prompt| prompt.id() == task.prompt_id)
        {
            self.owned
                .fail_local_prompt_without_advance_with_termination_if_matches(
                    &task.room_id,
                    &task.agent_id,
                    None,
                    Some(&task.prompt_id),
                    None,
                )?;
        }
        Ok(())
    }
}

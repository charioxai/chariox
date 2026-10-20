//! MP-08 / MP-10 / MP-11: retain the cause when an admitted provider dispatch fails.

use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

impl KernelRuntimeOwnedState {
    pub(super) fn block_failed_prompt_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: Option<&str>,
        diagnostic: &str,
    ) -> Result<(), DaemonError> {
        if !self.config_projection.snapshot().room_agent_tools {
            return Ok(());
        }
        let task = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|task| {
                task.prompt_id == prompt
                    && task.state == ExecutionState::Working
                    && run.is_none_or(|run| {
                        task.provider_run_id.as_deref().is_none_or(|id| id == run)
                    })
            });
        if let Some(task) = task {
            let diagnostic = crate::provider::sanitize_provider_diagnostic(diagnostic);
            self.durable_state_store.agent_lifecycle(Operation::Block {
                task: task.task_id,
                prompt: task.prompt_id,
                reason: format!(
                    "{}. Check the result before using Resume to retry, or Cancel.",
                    diagnostic.trim_end_matches('.')
                ),
            })?;
        }
        Ok(())
    }
}

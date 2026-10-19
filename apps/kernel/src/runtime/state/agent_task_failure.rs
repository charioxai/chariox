//! MP-08 / MP-10 / MP-11: retain the cause when an admitted provider dispatch fails.

use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

impl KernelRuntimeOwnedState {
    pub(super) fn block_failed_prompt_task(
        &self,
        room: &str,
        agent: &str,
        prompt: &str,
        run: &str,
        diagnostic: &str,
    ) -> Result<(), DaemonError> {
        let task = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|task| {
                task.prompt_id == prompt
                    && task.state == ExecutionState::Working
                    && task.provider_run_id.as_deref().is_none_or(|id| id == run)
            });
        let Some(task) = task else {
            return Ok(());
        };
        // A plain turn that owns nothing fails as it does without a ledger:
        // the failure notice is its whole outcome. Only a task holding an
        // obligation or a wait needs the owner to decide.
        if task.wait.is_none()
            && !task
                .obligations
                .iter()
                .any(|o| matches!(o.status.as_str(), "open" | "settling" | "failed"))
        {
            self.durable_state_store
                .agent_lifecycle(Operation::Settle {
                    room: room.into(),
                    agent: agent.into(),
                    prompt: prompt.into(),
                    run: run.into(),
                    cancelled: true,
                    now: crate::session::unix_epoch_ms(),
                })?;
            return Ok(());
        }
        {
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

//! MP-08 / MP-10 / MP-11: retain the cause when an admitted provider dispatch fails.

use super::*;
use crate::durable_state::agent_lifecycle::{ExecutionState, Operation};

impl KernelRuntimeOwnedState {
    pub(super) fn settle_failed_delegate_start(
        &self,
        room: &str,
        agent: &str,
    ) -> Result<(), DaemonError> {
        self.durable_state_store
            .agent_lifecycle(Operation::DelegateUnavailable {
                room: room.into(),
                agent: agent.into(),
                reason: crate::durable_state::agent_lifecycle::DelegateGoneReason::FailedStart,
                now: crate::session::unix_epoch_ms(),
            })?;
        Ok(())
    }

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
            if let Some(event) = self
                .durable_state_store
                .agent_event_for_prompt(room, agent, prompt)?
            {
                if event.reply_requested {
                    self.settle_failed_delegate_start(room, agent)?;
                }
            }
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

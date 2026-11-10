//! A cancel that an agent issues (a peer, a parent) ends the target's task, not the target: unlike
//! the owner's stop, later messages still run. The task is marked before the cancellation starts,
//! so no delivery sees the stopped task as an owner stop in between.
use super::*;
use crate::durable_state::agent_lifecycle::Operation;

impl KernelRuntimeState {
    /// Marks the task of `agent`'s running turn (or of `expected_prompt`) as cancelled by an
    /// agent and returns it, or `None` when the turn has no task. Withdraw the mark with
    /// [`Self::withdraw_agent_cancel`] when the cancellation then fails.
    pub(crate) fn mark_agent_cancel(
        &self,
        room: &str,
        agent: &str,
        expected_prompt: Option<&str>,
    ) -> Result<Option<String>, DaemonError> {
        let owned = &self.owned;
        let prompt = match expected_prompt {
            Some(prompt) => prompt.to_string(),
            None => {
                let session = owned.session_store.get_session(room)?;
                let Some(active) = owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, agent)
                else {
                    return Ok(None);
                };
                active.id().to_string()
            }
        };
        let Some(task) = owned
            .durable_state_store
            .agent_turn_or_continuation_task(room, agent, &prompt)?
        else {
            return Ok(None);
        };
        owned
            .durable_state_store
            .agent_lifecycle(Operation::MarkAgentCancel {
                task: task.task_id.clone(),
                by_agent: true,
            })?;
        Ok(Some(task.task_id))
    }

    pub(crate) fn withdraw_agent_cancel(&self, task: &str) {
        let withdrawn =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::MarkAgentCancel {
                    task: task.into(),
                    by_agent: false,
                });
        if let Err(error) = withdrawn {
            crate::logging::warn_with_fields(
                "daemon.agent_task",
                "agent cancel mark withdrawal failed",
                serde_json::json!({"task_id": task, "error": error.to_string()}),
            );
        }
    }
}

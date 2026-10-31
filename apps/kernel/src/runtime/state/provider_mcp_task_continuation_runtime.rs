//! MP-08 / MP-10 / MP-11: only a home-admitted intent may replay after catalog reload.
use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_catalog_task_continuation(
        &self,
        continuation: PendingMcpContinuation,
        previous_provider_run_id: Option<String>,
        hidden_system_context: String,
    ) -> Result<(), DaemonError> {
        // Only the exact source task's committed intent can replay a turn.
        // Refreshing a catalog never clears an owner-visible blocked task.
        let source_task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(&continuation.session_id), Some(&continuation.agent_id))?
            .into_iter()
            .find(|task| continuation.source_prompt_id.as_deref() == Some(task.prompt_id.as_str()));
        let Some(task) = source_task else {
            // MP-08/MP-10/MP-11: a worker cannot mint a home task or
            // replay an unbound prompt. Surface the lost intent through
            // the existing terminal/history projection instead of silence.
            self.report_lost_catalog_continuation(&continuation, previous_provider_run_id);
            return Err(crate::durable_state::agent_lifecycle::error(
                "catalog continuation home-owned task intent is missing",
            ));
        };
        if task.state == crate::durable_state::agent_lifecycle::ExecutionState::Working
            && task.wait.is_none()
            && task.pending_prompt_id.as_deref()
                == Some(format!("task-catalog:{}", task.prompt_id).as_str())
        {
            let attachment = self.ensure_mcp_continuation_attachment(&continuation)?;
            self.dispatch_task_continuation_with_context(
                &task,
                &attachment,
                continuation.previous_prompt,
                Some(hidden_system_context),
            )
            .await?;
            return Ok(());
        }
        // The agent was told the request would continue. An owner hold keeps
        // it from doing so; any other task that cannot resume breaks that
        // promise and must say so instead of leaving the Room idle.
        let owner_hold = matches!(
            task.state,
            crate::durable_state::agent_lifecycle::ExecutionState::Blocked
                | crate::durable_state::agent_lifecycle::ExecutionState::Cancelled
        ) || task.wait.is_some()
            || task.pending_prompt_id.is_some();
        if !owner_hold {
            self.report_lost_catalog_continuation(&continuation, previous_provider_run_id);
            return Err(crate::durable_state::agent_lifecycle::error(
                "catalog continuation task is no longer waiting for its refresh",
            ));
        }
        Ok(())
    }

    pub(super) fn report_lost_catalog_continuation(
        &self,
        continuation: &PendingMcpContinuation,
        previous_provider_run_id: Option<String>,
    ) {
        // Keep the source run ID: home may still be bound to that run.
        let run = previous_provider_run_id
            .or_else(|| {
                self.owned
                    .provider_store
                    .get_run_for_agent(&continuation.session_id, &continuation.agent_id)
                    .map(|run| run.id().to_string())
            })
            .unwrap_or_else(|| "kernel-catalog-reload".into());
        let message = "Tools were refreshed, but Chariox could not safely continue the interrupted request. Send the request again.";
        self.owned.record_provider_failure_output(
            &continuation.session_id,
            &run,
            &continuation.agent_id,
            message,
        );
    }
}

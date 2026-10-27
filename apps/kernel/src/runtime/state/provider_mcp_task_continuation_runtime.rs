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
        }
        Ok(())
    }
}

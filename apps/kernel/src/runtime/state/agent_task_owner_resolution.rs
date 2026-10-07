//! MP-08 / MP-09 / MP-10 / MP-11 A02: explicit owner disposition and visible failure.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, Operation, Outcome};

impl KernelRuntimeState {
    pub(super) async fn resolve_agent_task_owner_action(
        &self,
        task: &AgentTaskExecution,
        resume: bool,
    ) {
        let next = match self
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::OwnerResponse {
                task: task.task_id.clone(),
                revision: task.blocked_revision,
                resume,
                now: crate::session::unix_epoch_ms(),
            }) {
            Ok(Outcome::Task(next)) => next,
            Err(error) => {
                self.record_agent_task_owner_failure(task, &error);
                let _ = self.owned.session_snapshot(&task.room_id);
                return;
            }
            Ok(_) => unreachable!(),
        };
        let result = if resume {
            async {
                let agent = self.owned.agent_store.get_agent(&next.agent_id)?;
                let attachment = self.ensure_agent_message_attachment(&next.room_id, &agent)?;
                Box::pin(self.dispatch_task_continuation(&next, &attachment,
                    "Owner explicitly resumed this task. Reconcile retained obligations before continuing.".into())).await
            }.await
        } else {
            Box::pin(self.cancel_agent_task_resources(&next)).await
        };
        if let Err(error) = result {
            self.record_agent_task_owner_failure(&next, &error);
            if resume {
                // Preserve the exact current task if it raced a later disposition.
                // A failed continuation has no executor and cannot remain Working.
                if let Err(block_error) =
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Block {
                            task: next.task_id.clone(),
                            prompt: next.prompt_id.clone(),
                            reason: format!(
                                "Owner action failed: {}",
                                crate::secret_redaction::redact_secrets(&error.to_string())
                            ),
                        })
                {
                    tracing::warn!(error=%crate::secret_redaction::redact_secrets(&block_error.to_string()),
                        task_id=%next.task_id, "MP-08/MP-09/MP-10/MP-11 A02: later task state retained after failed owner continuation");
                }
            }
        }
        let _ = self.owned.session_snapshot(&next.room_id);
    }
    fn record_agent_task_owner_failure(&self, task: &AgentTaskExecution, error: &DaemonError) {
        self.owned.record_notice_for_agent(&task.room_id, None, Some(&task.agent_id),
            self.owned.attachment_store.list_session_attachment_ids(&task.room_id),
            format!("Owner action failed for task {}: {}. Recheck its current blocked interaction before retrying.",
                task.task_id, crate::secret_redaction::redact_secrets(&error.to_string())));
        self.owned
            .terminal_stream
            .notify_terminal_projection_change(&task.room_id);
    }
}

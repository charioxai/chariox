//! MP-08 / MP-09 / MP-10 / MP-11 A02: bounded public task answer projection.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
impl KernelRuntimeOwnedState {
    pub(super) fn public_agent_task_answer(
        &self,
        task: &AgentTaskExecution,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        if task.state != ExecutionState::Done {
            return Ok(None);
        }
        let entries = self
            .operational_history_store
            .load_session_history_entries(&task.room_id, Some(&task.agent_id))?;
        // A later task may reuse the provider run. Never borrow its answer.
        let key = format!("prompt:{}", task.prompt_id);
        let start = entries.iter().position(|e| {
            e.kind == crate::history::SessionHistoryEntryKind::UserPrompt
                && e.merge_key.as_deref() == Some(&key)
        });
        let excerpt = start
            .map(|index| {
                entries
                    .iter()
                    .skip(index + 1)
                    .take_while(|e| {
                        e.kind != crate::history::SessionHistoryEntryKind::UserPrompt
                            || e.merge_key
                                .as_deref()
                                .is_some_and(|k| k.starts_with("steer:"))
                    })
                    .filter(|e| {
                        e.kind == crate::history::SessionHistoryEntryKind::ProviderOutput
                            && e.provider_run_id == task.provider_run_id
                    })
                    .map(|e| e.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .chars()
                    .take(1_024)
                    .collect::<String>()
            })
            .unwrap_or_default();
        let mut answer = serde_json::json!({"agent_id":task.agent_id,"task_id":task.task_id,"prompt_id":task.prompt_id,"excerpt":excerpt});
        crate::secret_redaction::redact_json_secrets(&mut answer);
        Ok(Some(answer))
    }
}

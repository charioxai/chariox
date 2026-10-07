//! MP-08 / MP-09 / MP-10 / MP-11 A02: bounded public task answer projection.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState};
impl KernelRuntimeOwnedState {
    pub(super) fn record_agent_artifact_progress(
        &self,
        room: &str,
        agent: &str,
        prompt: &crate::session::PromptQueueItem,
        run: &str,
    ) -> Result<(), DaemonError> {
        let Some(snapshot) = self
            .completed_git_turn_snapshots
            .resolve(room, agent, Some(prompt.id()))
            .filter(|s| s.before.prompt_id == prompt.id() && s.before.provider_run_id == run)
        else {
            return Ok(());
        };
        if snapshot.before.head_sha == snapshot.after.head_sha
            && snapshot.before.status_fingerprint == snapshot.after.status_fingerprint
            && snapshot.before.workspace_live_sync_file_snapshots
                == snapshot.after.workspace_live_sync_file_snapshots
        {
            return Ok(());
        }
        let Some(task) = self
            .durable_state_store
            .agent_tasks(Some(room), Some(agent))?
            .into_iter()
            .find(|t| {
                t.prompt_id == prompt.id()
                    && t.provider_run_id.as_deref() == Some(run)
                    && t.state == ExecutionState::Working
                    && t.pending_prompt_id.is_none()
            })
        else {
            return Ok(());
        };
        use sha2::{Digest, Sha256};
        let normalized = serde_json::to_vec(&(
            &snapshot.after.head_sha,
            &snapshot.after.status_fingerprint,
            &snapshot.after.workspace_live_sync_file_snapshots,
        ))
        .map_err(|_| {
            crate::durable_state::agent_lifecycle::error("artifact receipt normalization failed")
        })?;
        let receipt = format!("git-artifact:{:x}", Sha256::digest(normalized));
        self.durable_state_store.agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::Progress {
                task: task.task_id,
                prompt: prompt.id().into(),
                receipt,
                now: crate::session::unix_epoch_ms(),
            },
        )?;
        Ok(())
    }

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
                            || e.merge_key.as_deref().is_some_and(|k| {
                                k.starts_with(crate::history::STEERING_PROMPT_MERGE_KEY_PREFIX)
                            })
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

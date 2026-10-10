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
            .load_session_history_entries(&task.room_id, Some(&task.agent_id))?
            .into_iter()
            .map(|entry| {
                self.room_secret_observations
                    .protect_transcript_entry(entry)
            })
            .collect::<Vec<_>>();
        // Output entries are streamed deltas; the conclusion is at the end.
        let output = task_public_outputs(
            &entries,
            &task.prompt_id,
            task_answer_history_run(task.provider_run_id.as_deref()),
        )
        .concat();
        let excerpt = answer_tail(output.trim(), 1_024);
        let mut answer = serde_json::json!({"agent_id":task.agent_id,"task_id":task.task_id,"prompt_id":task.prompt_id,"excerpt":excerpt});
        crate::secret_redaction::redact_json_secrets(&mut answer);
        Ok(Some(answer))
    }
}

// Leased history retains the worker run ID. Resolve it from the task's durable
// execution binding, so reconciliation does not depend on the current lease or
// active run. This is a history lookup only; the task's authority stays projected.
fn task_answer_history_run(run: Option<&str>) -> Option<&str> {
    run.map(|run| {
        run.strip_prefix("leased:")
            .and_then(|binding| binding.split_once(':'))
            .filter(|(lease, worker)| !lease.is_empty() && !worker.is_empty())
            .map_or(run, |(_, worker)| worker)
    })
}

fn answer_tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.into();
    }
    let tail = text.chars().skip(count - limit + 1).collect::<String>();
    format!("…{tail}")
}

// Steering belongs to the current turn; an independent prompt closes its answer boundary.
pub(super) fn task_public_outputs<'a>(
    entries: &'a [crate::history::SessionHistoryEntry],
    prompt: &str,
    run: Option<&str>,
) -> Vec<&'a str> {
    use crate::history::SessionHistoryEntryKind as Kind;
    let key = format!("prompt:{prompt}");
    let Some(start) = entries
        .iter()
        .position(|e| e.kind == Kind::UserPrompt && e.merge_key.as_deref() == Some(&key))
    else {
        return vec![];
    };
    entries
        .iter()
        .skip(start + 1)
        .take_while(|e| {
            e.kind != Kind::UserPrompt
                || e.merge_key.as_deref().is_some_and(|key| {
                    key.starts_with(crate::history::STEERING_PROMPT_MERGE_KEY_PREFIX)
                })
        })
        .filter(|e| {
            e.kind == Kind::ProviderOutput && run.is_some() && e.provider_run_id.as_deref() == run
        })
        .map(|e| e.text.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a02_public_answer_survives_steering_without_borrowing_later_tasks() {
        let entry = |kind: &str, key: Option<&str>, run: Option<&str>, text: &str| {
            serde_json::from_value(serde_json::json!({"session_id":"room", "kind":kind,
                "merge_key":key,"provider_run_id":run,"text":text,"timestamp_ms":1}))
            .unwrap()
        };
        let entries = vec![
            entry("user_prompt", Some("prompt:current"), None, "task"),
            entry(
                "provider_output",
                None,
                Some("run"),
                "answer before steering",
            ),
            entry(
                "user_prompt",
                Some("steering-prompt:urgent"),
                None,
                "urgent",
            ),
            entry(
                "provider_output",
                None,
                Some("foreign-run"),
                "foreign answer",
            ),
            entry("user_prompt", Some("prompt:later"), None, "later task"),
            entry("provider_output", None, Some("run"), "later answer"),
        ];
        assert_eq!(
            task_public_outputs(&entries, "current", Some("run")),
            vec!["answer before steering"]
        );
        assert!(task_public_outputs(&entries, "absent", Some("run")).is_empty());
        assert!(task_public_outputs(&entries, "current", None).is_empty());
    }
    // MP-08/MP-10/MP-11 R947: persisted completion/sweep lookup uses the
    // task's old worker run even after another lease or turn becomes current.
    #[test]
    fn a10_public_answer_resolves_durable_worker_run_without_live_binding() {
        assert_eq!(
            task_answer_history_run(Some("leased:old-lease:old-run")),
            Some("old-run")
        );
        assert_eq!(
            task_answer_history_run(Some("leased:old-lease:worker:run")),
            Some("worker:run")
        );
        for run in ["local-run", "leased:", "leased::worker", "leased:lease:"] {
            assert_eq!(task_answer_history_run(Some(run)), Some(run));
        }
        assert_eq!(task_answer_history_run(None), None);
        let entry = |kind: &str, key: Option<&str>, run: Option<&str>, text: &str| {
            serde_json::from_value(serde_json::json!({"session_id":"room", "kind":kind,
                "merge_key":key,"provider_run_id":run,"text":text,"timestamp_ms":1}))
            .unwrap()
        };
        let entries = vec![
            entry("user_prompt", Some("prompt:completed"), None, "task"),
            entry(
                "provider_output",
                None,
                Some("old-run"),
                "verified worker answer",
            ),
            entry(
                "provider_output",
                None,
                Some("current-run"),
                "foreign answer",
            ),
            entry("user_prompt", Some("prompt:later"), None, "later task"),
            entry("provider_output", None, Some("old-run"), "later answer"),
        ];
        assert_eq!(
            task_public_outputs(
                &entries,
                "completed",
                task_answer_history_run(Some("leased:old-lease:old-run"))
            ),
            vec!["verified worker answer"]
        );
    }

    #[test]
    fn a02_public_answer_keeps_streamed_conclusion() {
        let streamed = ["I", "’ll", " check.", " Done: all tools exist."].concat();
        assert_eq!(
            answer_tail(&streamed, 64),
            "I’ll check. Done: all tools exist."
        );
        let tail = answer_tail(&streamed, 12);
        assert_eq!(tail, "…ools exist.");
        assert_eq!(tail.chars().count(), 12);
    }
}

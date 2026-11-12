//! An agent's wait is a row of its own transcript. The kernel writes the row when the wait
//! starts and rewrites that same row when the agent wakes, with what woke it and how long it
//! waited, so every client shows the wait from the transcript instead of deriving it.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState, InboxEvent};
use crate::history::SessionHistoryEntry;
use crate::terminal::{TerminalOutputAppend, TerminalOutputKind};

const WAIT_TOOL: &str = "chariox.wait";

/// Writing a row and closing it are one decision about the wait at a time: a wake that lands while
/// the row is being written finds the row, or the writer finds the wake.
static WAIT_ROWS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn wait_rows_guard() -> std::sync::MutexGuard<'static, ()> {
    WAIT_ROWS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_merge_key(task_id: &str, started_at_ms: u64) -> String {
    format!("wait:{task_id}:{started_at_ms}")
}

impl KernelRuntimeOwnedState {
    /// The task's committed wait, as a running row. The settlement that committed the wait is a
    /// snapshot, so the row is written only for a wait that is still going on: a wake that landed
    /// since has moved the task on, or ended the wait through an event that names no task.
    pub(super) fn record_wait_started(&self, task: &AgentTaskExecution) {
        let Some(wait) = task.wait.as_ref() else {
            return;
        };
        let _rows = wait_rows_guard();
        let still_waiting = self
            .durable_state_store
            .agent_task(&task.task_id)
            .ok()
            .flatten()
            .is_some_and(|current| {
                current.state == ExecutionState::Waiting
                    && current.wait.as_ref().map(|w| w.started_at_ms) == Some(wait.started_at_ms)
            });
        if !still_waiting {
            return;
        }
        let ended_by = self
            .durable_state_store
            .agent_work_inbox(&task.room_id, &task.agent_id, wait.inbox_cursor, None)
            .unwrap_or_default()
            .into_iter()
            .find(|event| {
                event.attempted_at_ms.is_some()
                    && event.state != "pending"
                    && self.event_reports_wait_source(task, event)
            });
        let key = wait_merge_key(&task.task_id, wait.started_at_ms);
        self.write_task_wait_row(
            task,
            &key,
            serde_json::json!({
                "tool": WAIT_TOOL, "id": key, "status": "running", "background": true,
                "input": {"reason": task.reason, "deadline_ms": wait.deadline_ms},
                "started_at_ms": wait.started_at_ms, "finished_at_ms": null,
            }),
        );
        if let Some(event) = ended_by {
            self.close_wait_row(task, &key, &event.kind, &event.source_id);
        }
    }

    /// The delivery attempt of `event` (and the rows that share its prompt) ends the waits it ends:
    /// those whose task left Waiting, and those whose source reports its end through an event that
    /// names no task (a child finishing). A message, or an urgent event steering another turn,
    /// leaves the wait going on.
    pub(super) fn record_woken_waits(&self, before: &[AgentTaskExecution], event: &InboxEvent) {
        let _rows = wait_rows_guard();
        let still_waiting = self.waiting_tasks(&event.room_id, &event.agent_id);
        let batch = event
            .prompt_id
            .as_deref()
            .and_then(|prompt| {
                self.durable_state_store
                    .agent_events_for_prompt(&event.room_id, &event.agent_id, prompt)
                    .ok()
            })
            .filter(|batch| !batch.is_empty())
            .unwrap_or_else(|| vec![event.clone()]);
        for task in before {
            let Some(wait) = task.wait.as_ref() else {
                continue;
            };
            let kept = still_waiting.iter().any(|t| {
                t.task_id == task.task_id
                    && t.wait.as_ref().map(|w| w.started_at_ms) == Some(wait.started_at_ms)
            });
            let ended_by = if kept {
                batch
                    .iter()
                    .find(|e| self.event_reports_wait_source(task, e))
            } else {
                Some(event)
            };
            if let Some(ended_by) = ended_by {
                self.close_wait_row(
                    task,
                    &wait_merge_key(&task.task_id, wait.started_at_ms),
                    &ended_by.kind,
                    &ended_by.source_id,
                );
            }
        }
    }

    /// A refused wake did not run: the waits it closed are going on again.
    pub(super) fn record_wake_refused(&self, event: &InboxEvent) {
        let _rows = wait_rows_guard();
        for task in self.waiting_tasks(&event.room_id, &event.agent_id) {
            let Some(wait) = task.wait.as_ref() else {
                continue;
            };
            self.reopen_wait_row(
                &task,
                &wait_merge_key(&task.task_id, wait.started_at_ms),
                event,
            );
        }
    }

    /// The event is the end of a source the task's wait is on.
    fn event_reports_wait_source(&self, task: &AgentTaskExecution, event: &InboxEvent) -> bool {
        let Some(wait) = task.wait.as_ref() else {
            return false;
        };
        event.is_source_outcome()
            && self
                .durable_state_store
                .agent_registrations(&task.task_id)
                .unwrap_or_default()
                .iter()
                .any(|registration| {
                    wait.registration_ids.contains(&registration.id)
                        && event.reports_source(&registration.source_id)
                })
    }

    /// A task that ended without a wake (the owner stopped it) closes the row of the wait it was in.
    pub(super) fn record_wait_cancelled(&self, task: &AgentTaskExecution) {
        self.close_wait_row(
            task,
            &format!("wait:{}:", task.task_id),
            "cancelled",
            &task.task_id,
        );
    }

    /// Rewrites the newest row whose key starts with `prefix` as completed, in place, unless it already is.
    fn close_wait_row(&self, task: &AgentTaskExecution, prefix: &str, kind: &str, source: &str) {
        let Some((key, run, mut row)) = self.latest_wait_row(task, prefix) else {
            return;
        };
        if row["status"] != "running" {
            return;
        }
        row["status"] = "completed".into();
        row["finished_at_ms"] = crate::session::unix_epoch_ms().into();
        row["woken_by"] = serde_json::json!({"kind": kind, "source_id": source});
        self.write_wait_row(task, &run, &key, row);
    }

    /// Rewrites a row that `event` closed as running again, in place.
    fn reopen_wait_row(&self, task: &AgentTaskExecution, key: &str, event: &InboxEvent) {
        let Some((key, run, mut row)) = self.latest_wait_row(task, key) else {
            return;
        };
        if row["status"] != "completed"
            || row["woken_by"]["kind"] != event.kind.as_str()
            || row["woken_by"]["source_id"] != event.source_id.as_str()
        {
            return;
        }
        row["status"] = "running".into();
        row["finished_at_ms"] = serde_json::Value::Null;
        row.as_object_mut().map(|row| row.remove("woken_by"));
        self.write_wait_row(task, &run, &key, row);
    }

    /// The newest row whose key starts with `prefix`: its key, run and content.
    fn latest_wait_row(
        &self,
        task: &AgentTaskExecution,
        prefix: &str,
    ) -> Option<(String, String, serde_json::Value)> {
        let event = self
            .operational_history_store
            .latest_transcript_by_merge_key_prefix(&task.room_id, &task.agent_id, prefix)
            .ok()??;
        let entry = event.to_session_history_entry()?;
        let row = serde_json::from_str(&entry.text).ok()?;
        Some((
            entry.merge_key.clone()?,
            entry.provider_run_id.clone()?,
            row,
        ))
    }

    /// The agent's tasks whose wait is committed: what a wake can end.
    pub(super) fn waiting_tasks(&self, room: &str, agent: &str) -> Vec<AgentTaskExecution> {
        self.durable_state_store
            .agent_tasks(Some(room), Some(agent))
            .unwrap_or_default()
            .into_iter()
            .filter(|task| task.state == ExecutionState::Waiting && task.wait.is_some())
            .collect()
    }

    /// The row of a wait is written in the run the task waits in.
    fn write_task_wait_row(&self, task: &AgentTaskExecution, key: &str, row: serde_json::Value) {
        let Some(run) = task.provider_run_id.as_deref() else {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "wait row not written: the task has no provider run",
                serde_json::json!({"task_id": task.task_id, "agent_id": task.agent_id}),
            );
            return;
        };
        self.write_wait_row(task, run, key, row);
    }

    fn write_wait_row(
        &self,
        task: &AgentTaskExecution,
        run: &str,
        key: &str,
        row: serde_json::Value,
    ) {
        let text = row.to_string();
        let entry = SessionHistoryEntry::provider_output(
            &task.room_id,
            run,
            Some(&task.agent_id),
            TerminalOutputKind::ProviderTool,
            Some(key.to_owned()),
            text.clone(),
        );
        let Some(event) = self.upsert_operational_history_row(&entry, key) else {
            return;
        };
        let recipients = self.agent_trace_recipient_attachment_ids(
            &task.room_id,
            Some(&task.agent_id),
            self.attachment_store
                .list_session_attachment_ids(&task.room_id),
        );
        let recipients = self.with_metaagent_trace_recipient_ids(
            &task.room_id,
            Some(&task.agent_id),
            recipients,
        );
        self.terminal_stream
            .fan_out_outputs(vec![TerminalOutputAppend {
                session_id: task.room_id.clone(),
                provider_run_id: run.to_owned(),
                agent_id: Some(task.agent_id.clone()),
                prompt_id: event.prompt_id.clone(),
                prompt_origin: None,
                source_attachment_id: None,
                kind: TerminalOutputKind::ProviderTool,
                merge_key: Some(key.to_owned()),
                sequence: Some(event.sequence),
                recipient_attachment_ids: recipients.into(),
                bytes: text.into_bytes(),
            }]);
        self.notify_metaagent_trace_activity(&task.room_id, Some(&task.agent_id));
    }
}

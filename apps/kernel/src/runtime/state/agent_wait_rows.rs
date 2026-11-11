//! An agent's wait is a row of its own transcript. The kernel writes the row when the wait
//! starts and rewrites that same row when the agent wakes, with what woke it and how long it
//! waited, so every client shows the wait from the transcript instead of deriving it.
use super::*;
use crate::durable_state::agent_lifecycle::{AgentTaskExecution, ExecutionState, InboxEvent};
use crate::history::SessionHistoryEntry;
use crate::terminal::{TerminalOutputAppend, TerminalOutputKind};

const WAIT_TOOL: &str = "chariox.wait";

fn wait_merge_key(task_id: &str, started_at_ms: u64) -> String {
    format!("wait:{task_id}:{started_at_ms}")
}

impl KernelRuntimeOwnedState {
    /// The task's committed wait, as a running row.
    pub(super) fn record_wait_started(&self, task: &AgentTaskExecution) {
        let Some(wait) = task.wait.as_ref() else {
            return;
        };
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
    }

    /// An event that starts a turn for the agent ends the waits it was in: the rows of the waits `before` held
    /// close, whatever the ledger still says of those tasks (the woken turn runs as a task of its own).
    pub(super) fn record_woken_waits(&self, before: &[AgentTaskExecution], event: &InboxEvent) {
        for task in before {
            let Some(wait) = task.wait.as_ref() else {
                continue;
            };
            self.close_wait_row(
                task,
                &wait_merge_key(&task.task_id, wait.started_at_ms),
                &event.kind,
                &event.source_id,
            );
        }
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
        let Ok(Some(event)) = self
            .operational_history_store
            .latest_transcript_by_merge_key_prefix(&task.room_id, &task.agent_id, prefix)
        else {
            return;
        };
        let Some(entry) = event.to_session_history_entry() else {
            return;
        };
        let (Some(key), Some(run), Ok(mut row)) = (
            entry.merge_key.clone(),
            entry.provider_run_id.clone(),
            serde_json::from_str::<serde_json::Value>(&entry.text),
        ) else {
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

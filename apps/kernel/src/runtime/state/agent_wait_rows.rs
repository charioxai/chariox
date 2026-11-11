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
        self.write_wait_row(
            task,
            &key,
            serde_json::json!({
                "tool": WAIT_TOOL, "id": key, "status": "running", "background": true,
                "input": {"reason": task.reason, "deadline_ms": wait.deadline_ms},
                "started_at_ms": wait.started_at_ms, "finished_at_ms": null,
            }),
        );
    }

    /// Closes the rows of the waits `before` held that `event` ended by waking their tasks.
    pub(super) fn record_woken_waits(&self, before: &[AgentTaskExecution], event: &InboxEvent) {
        for task in before {
            let still_waiting = self
                .durable_state_store
                .agent_task(&task.task_id)
                .ok()
                .flatten()
                .is_some_and(|now| now.state == ExecutionState::Waiting);
            if !still_waiting {
                self.record_wait_ended(task, &event.kind, &event.source_id);
            }
        }
    }

    /// Rewrites the row of the wait `task` held when it ended.
    pub(super) fn record_wait_ended(&self, task: &AgentTaskExecution, kind: &str, source: &str) {
        let Some(wait) = task.wait.as_ref() else {
            return;
        };
        let key = wait_merge_key(&task.task_id, wait.started_at_ms);
        let finished = crate::session::unix_epoch_ms();
        self.write_wait_row(
            task,
            &key,
            serde_json::json!({
                "tool": WAIT_TOOL, "id": key, "status": "completed", "background": true,
                "input": {"reason": task.reason, "deadline_ms": wait.deadline_ms},
                "started_at_ms": wait.started_at_ms, "finished_at_ms": finished,
                "woken_by": {"kind": kind, "source_id": source},
            }),
        );
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

    fn write_wait_row(&self, task: &AgentTaskExecution, key: &str, row: serde_json::Value) {
        let Some(run) = task.provider_run_id.as_deref() else {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "wait row not written: the task has no provider run",
                serde_json::json!({"task_id": task.task_id, "agent_id": task.agent_id}),
            );
            return;
        };
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

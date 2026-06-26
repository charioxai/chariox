use serde::{Deserialize, Serialize};

use super::unix_epoch_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaagentTaskStatus {
    Active,
    Paused,
    Blocked,
    Completed,
    Aborted,
}

impl MetaagentTaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Aborted)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaagentTask {
    task_id: String,
    metaagent_id: String,
    status: MetaagentTaskStatus,
    task_markdown: String,
    plan_markdown: String,
    revision: u64,
    created_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    aborted_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completion_summary: Option<String>,
}

impl MetaagentTask {
    pub fn new(
        task_id: impl Into<String>,
        metaagent_id: impl Into<String>,
        task_markdown: impl Into<String>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            task_id: task_id.into(),
            metaagent_id: metaagent_id.into(),
            status: MetaagentTaskStatus::Active,
            task_markdown: task_markdown.into(),
            plan_markdown: String::new(),
            revision: 1,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
            blocked_reason: None,
            aborted_reason: None,
            completion_summary: None,
        }
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn metaagent_id(&self) -> &str {
        &self.metaagent_id
    }

    pub fn status(&self) -> MetaagentTaskStatus {
        self.status
    }

    pub fn task_markdown(&self) -> &str {
        &self.task_markdown
    }

    pub fn plan_markdown(&self) -> &str {
        &self.plan_markdown
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub fn completed_at_ms(&self) -> Option<u64> {
        self.completed_at_ms
    }

    pub fn blocked_reason(&self) -> Option<&str> {
        self.blocked_reason.as_deref()
    }

    pub fn aborted_reason(&self) -> Option<&str> {
        self.aborted_reason.as_deref()
    }

    pub fn completion_summary(&self) -> Option<&str> {
        self.completion_summary.as_deref()
    }

    pub fn update_task_markdown(&mut self, task_markdown: impl Into<String>) {
        self.task_markdown = task_markdown.into();
        self.reopen_if_terminal();
        self.touch();
    }

    pub fn restart(&mut self, task_id: impl Into<String>, task_markdown: impl Into<String>) {
        let now = unix_epoch_ms();
        self.task_id = task_id.into();
        self.status = MetaagentTaskStatus::Active;
        self.task_markdown = task_markdown.into();
        self.plan_markdown.clear();
        self.revision = self.revision.saturating_add(1);
        self.created_at_ms = now;
        self.updated_at_ms = now;
        self.completed_at_ms = None;
        self.blocked_reason = None;
        self.aborted_reason = None;
        self.completion_summary = None;
    }

    pub fn update_plan_markdown(&mut self, plan_markdown: impl Into<String>) {
        self.plan_markdown = plan_markdown.into();
        self.reopen_if_terminal();
        self.touch();
    }

    pub fn set_status(&mut self, status: MetaagentTaskStatus) {
        self.status = status;
        if !status.is_terminal() {
            self.completed_at_ms = None;
            self.completion_summary = None;
            self.aborted_reason = None;
        }
        if status != MetaagentTaskStatus::Blocked {
            self.blocked_reason = None;
        }
        self.touch();
    }

    pub fn mark_completed(&mut self, summary: Option<String>) {
        self.status = MetaagentTaskStatus::Completed;
        self.completion_summary = summary;
        self.completed_at_ms = Some(unix_epoch_ms());
        self.blocked_reason = None;
        self.aborted_reason = None;
        self.touch();
    }

    pub fn mark_blocked(&mut self, reason: impl Into<String>) {
        self.status = MetaagentTaskStatus::Blocked;
        self.blocked_reason = Some(reason.into());
        self.completed_at_ms = None;
        self.completion_summary = None;
        self.aborted_reason = None;
        self.touch();
    }

    pub fn abort(&mut self, reason: Option<String>) {
        self.status = MetaagentTaskStatus::Aborted;
        self.aborted_reason = reason;
        self.completed_at_ms = None;
        self.completion_summary = None;
        self.blocked_reason = None;
        self.touch();
    }

    fn reopen_if_terminal(&mut self) {
        if self.status.is_terminal() {
            self.status = MetaagentTaskStatus::Active;
            self.completed_at_ms = None;
            self.completion_summary = None;
            self.aborted_reason = None;
        }
        if self.status == MetaagentTaskStatus::Blocked {
            self.status = MetaagentTaskStatus::Active;
            self.blocked_reason = None;
        }
    }

    fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at_ms = unix_epoch_ms();
    }
}

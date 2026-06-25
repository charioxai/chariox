use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

pub const DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS: u64 = 100;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowWatchdogPolicy {
    Skip,
    Queue,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowQueuedPromptSource {
    Manual,
    Watchdog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationInvocationEnvelope {
    pub publication_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_id: Option<String>,
    pub invocation_id: String,
    pub transport: String,
    pub endpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_ref: Option<String>,
    #[serde(
        default = "serde_json_null",
        skip_serializing_if = "serde_json_value_is_null"
    )]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(
        default = "serde_json_null",
        skip_serializing_if = "serde_json_value_is_null"
    )]
    pub caller: serde_json::Value,
}

impl WorkflowPublicationInvocationEnvelope {
    pub fn invocation_id(&self) -> &str {
        &self.invocation_id
    }
}

fn serde_json_null() -> serde_json::Value {
    serde_json::Value::Null
}

fn serde_json_value_is_null(value: &serde_json::Value) -> bool {
    value.is_null()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPromptQueueDefinition {
    id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    workflow_id: String,
    alias: String,
    priority: i32,
    enabled: bool,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl WorkflowPromptQueueDefinition {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        alias: impl Into<String>,
        priority: i32,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            alias: alias.into(),
            priority,
            enabled: true,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn default_queue(workflow_id: impl Into<String>) -> Self {
        let workflow_id = workflow_id.into();
        Self::new(format!("{workflow_id}:default"), workflow_id, "default", 0)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }
    pub fn alias(&self) -> &str {
        &self.alias
    }
    pub fn priority(&self) -> i32 {
        self.priority
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub fn set_alias(&mut self, alias: impl Into<String>) {
        self.alias = alias.into();
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_priority(&mut self, priority: i32) {
        self.priority = priority;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.updated_at_ms = unix_epoch_ms();
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowQueuedPromptStatus {
    Queued,
    Dispatching,
    Running,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowQueuedPrompt {
    id: String,
    queue_id: String,
    workflow_id: String,
    endpoint_id: String,
    prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
    source: WorkflowQueuedPromptSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    watchdog_id: Option<String>,
    status: WorkflowQueuedPromptStatus,
    created_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dispatched_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_run_id: Option<String>,
}

impl WorkflowQueuedPrompt {
    pub fn new(
        id: impl Into<String>,
        queue_id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        prompt: Option<String>,
        publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
        source: WorkflowQueuedPromptSource,
        watchdog_id: Option<String>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            queue_id: queue_id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            prompt,
            publication_invocation,
            source,
            watchdog_id,
            status: WorkflowQueuedPromptStatus::Queued,
            created_at_ms: now,
            updated_at_ms: now,
            dispatched_at_ms: None,
            workflow_run_id: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn queue_id(&self) -> &str {
        &self.queue_id
    }
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }
    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }
    pub fn publication_invocation(&self) -> Option<&WorkflowPublicationInvocationEnvelope> {
        self.publication_invocation.as_ref()
    }
    pub fn source(&self) -> WorkflowQueuedPromptSource {
        self.source
    }
    pub fn watchdog_id(&self) -> Option<&str> {
        self.watchdog_id.as_deref()
    }
    pub fn status(&self) -> WorkflowQueuedPromptStatus {
        self.status
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }
    pub fn dispatched_at_ms(&self) -> Option<u64> {
        self.dispatched_at_ms
    }
    pub fn workflow_run_id(&self) -> Option<&str> {
        self.workflow_run_id.as_deref()
    }

    pub fn set_prompt(&mut self, prompt: Option<String>) {
        self.prompt = prompt;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_queue_id(&mut self, queue_id: impl Into<String>) {
        self.queue_id = queue_id.into();
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn mark_dispatching(&mut self) {
        self.status = WorkflowQueuedPromptStatus::Dispatching;
        let now = unix_epoch_ms();
        self.updated_at_ms = now;
        self.dispatched_at_ms = Some(now);
    }

    pub fn mark_running(&mut self, workflow_run_id: impl Into<String>) {
        self.status = WorkflowQueuedPromptStatus::Running;
        self.workflow_run_id = Some(workflow_run_id.into());
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn mark_completed(&mut self) {
        self.status = WorkflowQueuedPromptStatus::Completed;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn mark_cancelled(&mut self) {
        self.status = WorkflowQueuedPromptStatus::Cancelled;
        self.updated_at_ms = unix_epoch_ms();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowWatchdogDefinition {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_id: Option<String>,
    enabled: bool,
    interval_seconds: u64,
    invocation_prompt: String,
    policy: WorkflowWatchdogPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_wakeups: Option<u64>,
    #[serde(default)]
    wakeups_executed: u64,
    next_run_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_run_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_workflow_run_id: Option<String>,
    #[serde(default)]
    pending_run: bool,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl WorkflowWatchdogDefinition {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        interval_seconds: u64,
        invocation_prompt: impl Into<String>,
        policy: WorkflowWatchdogPolicy,
        max_wakeups: Option<u64>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            queue_id: None,
            enabled: true,
            interval_seconds,
            invocation_prompt: invocation_prompt.into(),
            policy,
            max_wakeups,
            wakeups_executed: 0,
            next_run_at_ms: now.saturating_add(interval_seconds.saturating_mul(1000)),
            last_run_at_ms: None,
            last_status: None,
            last_error: None,
            last_workflow_run_id: None,
            pending_run: false,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }
    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }
    pub fn queue_id(&self) -> Option<&str> {
        self.queue_id.as_deref()
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn interval_seconds(&self) -> u64 {
        self.interval_seconds
    }
    pub fn invocation_prompt(&self) -> &str {
        &self.invocation_prompt
    }
    pub fn policy(&self) -> WorkflowWatchdogPolicy {
        self.policy
    }
    pub fn max_wakeups(&self) -> Option<u64> {
        self.max_wakeups
    }
    pub fn wakeups_executed(&self) -> u64 {
        self.wakeups_executed
    }
    pub fn next_run_at_ms(&self) -> u64 {
        self.next_run_at_ms
    }
    pub fn last_run_at_ms(&self) -> Option<u64> {
        self.last_run_at_ms
    }
    pub fn last_status(&self) -> Option<&str> {
        self.last_status.as_deref()
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn last_workflow_run_id(&self) -> Option<&str> {
        self.last_workflow_run_id.as_deref()
    }
    pub fn pending_run(&self) -> bool {
        self.pending_run
    }
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }
    pub fn updated_at_ms(&self) -> u64 {
        self.updated_at_ms
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_next_run_at_ms(&mut self, value: u64) {
        self.next_run_at_ms = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_queue_id(&mut self, value: Option<String>) {
        self.queue_id = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_run_at_ms(&mut self, value: Option<u64>) {
        self.last_run_at_ms = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_status(&mut self, value: Option<String>) {
        self.last_status = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_error(&mut self, value: Option<String>) {
        self.last_error = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_last_workflow_run_id(&mut self, value: Option<String>) {
        self.last_workflow_run_id = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_pending_run(&mut self, value: bool) {
        self.pending_run = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_max_wakeups(&mut self, value: Option<u64>) {
        self.max_wakeups = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_wakeups_executed(&mut self, value: u64) {
        self.wakeups_executed = value;
        self.updated_at_ms = unix_epoch_ms();
    }
}

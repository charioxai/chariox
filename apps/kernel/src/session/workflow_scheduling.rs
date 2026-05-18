use serde::{Deserialize, Serialize};

use super::types::unix_epoch_ms;

pub const DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS: u64 = 100;
pub const DEFAULT_WORKFLOW_LAUNCH_POLICY: WorkflowLaunchPolicy = WorkflowLaunchPolicy::Reject;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowWatchdogPolicy {
    Skip,
    Queue,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowLaunchPolicy {
    Reject,
    Queue,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuedWorkflowLaunchSource {
    Manual,
    Watchdog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedWorkflowLaunch {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    invocation_prompt: Option<String>,
    source: QueuedWorkflowLaunchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    watchdog_id: Option<String>,
    queued_at_ms: u64,
}

impl QueuedWorkflowLaunch {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        invocation_prompt: Option<String>,
        source: QueuedWorkflowLaunchSource,
        watchdog_id: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            invocation_prompt,
            source,
            watchdog_id,
            queued_at_ms: unix_epoch_ms(),
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
    pub fn invocation_prompt(&self) -> Option<&str> {
        self.invocation_prompt.as_deref()
    }
    pub fn source(&self) -> QueuedWorkflowLaunchSource {
        self.source
    }
    pub fn watchdog_id(&self) -> Option<&str> {
        self.watchdog_id.as_deref()
    }
    pub fn queued_at_ms(&self) -> u64 {
        self.queued_at_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowWatchdogDefinition {
    id: String,
    workflow_id: String,
    endpoint_id: String,
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

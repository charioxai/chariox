use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use croner::{
    parser::{CronParser, Seconds, Year},
    Cron,
};
use serde::{de, Deserialize, Deserializer, Serialize};

use super::types::unix_epoch_ms;

pub const DEFAULT_WORKFLOW_SCHEDULE_MAX_RUNS: u64 = 100;
pub const DEFAULT_WORKFLOW_WATCHDOG_MAX_WAKEUPS: u64 = DEFAULT_WORKFLOW_SCHEDULE_MAX_RUNS;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowScheduleOverlapPolicy {
    Skip,
    Queue,
}

pub type WorkflowWatchdogPolicy = WorkflowScheduleOverlapPolicy;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowQueuedPromptSource {
    Manual,
    #[serde(alias = "watchdog")]
    Scheduled,
    Event,
}

pub type WorkflowWatchdogDefinition = WorkflowScheduleDefinition;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowScheduleTrigger {
    Interval {
        every_seconds: u64,
    },
    Cron {
        expression: String,
        timezone: String,
    },
}

impl WorkflowScheduleTrigger {
    pub fn interval(every_seconds: u64) -> Self {
        Self::Interval { every_seconds }
    }

    pub fn cron(expression: impl Into<String>, timezone: impl Into<String>) -> Self {
        Self::Cron {
            expression: expression.into(),
            timezone: timezone.into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Interval { every_seconds } => {
                if *every_seconds == 0 {
                    Err("interval schedules require every_seconds greater than zero".to_string())
                } else {
                    Ok(())
                }
            }
            Self::Cron {
                expression,
                timezone,
            } => {
                parse_workflow_schedule_timezone(timezone)?;
                parse_workflow_schedule_cron(expression)?;
                Ok(())
            }
        }
    }

    pub fn next_run_after_ms(&self, after_ms: u64) -> Result<u64, String> {
        match self {
            Self::Interval { every_seconds } => {
                if *every_seconds == 0 {
                    return Err(
                        "interval schedules require every_seconds greater than zero".to_string()
                    );
                }
                Ok(after_ms.saturating_add(every_seconds.saturating_mul(1000)))
            }
            Self::Cron {
                expression,
                timezone,
            } => {
                let cron = parse_workflow_schedule_cron(expression)?;
                let timezone = parse_workflow_schedule_timezone(timezone)?;
                let start_utc = Utc
                    .timestamp_millis_opt(after_ms as i64)
                    .single()
                    .ok_or_else(|| "schedule start timestamp is out of range".to_string())?;
                let start_local = start_utc.with_timezone(&timezone);
                let next = cron
                    .find_next_occurrence(&start_local, false)
                    .map_err(|err| format!("invalid cron schedule: {err}"))?;
                Ok(next.with_timezone(&Utc).timestamp_millis().max(0) as u64)
            }
        }
    }

    pub fn preview_run_times_after_ms(
        &self,
        after_ms: u64,
        count: usize,
    ) -> Result<Vec<u64>, String> {
        let mut cursor = after_ms;
        let mut runs = Vec::with_capacity(count);
        for _ in 0..count {
            let next = self.next_run_after_ms(cursor)?;
            runs.push(next);
            cursor = next;
        }
        Ok(runs)
    }
}

fn parse_workflow_schedule_timezone(timezone: &str) -> Result<Tz, String> {
    timezone
        .trim()
        .parse::<Tz>()
        .map_err(|_| format!("invalid IANA timezone `{}`", timezone.trim()))
}

fn parse_workflow_schedule_cron(expression: &str) -> Result<Cron, String> {
    let expression = expression.trim();
    if expression.is_empty() {
        return Err("cron expression is required".to_string());
    }
    CronParser::builder()
        .seconds(Seconds::Required)
        .year(Year::Disallowed)
        .build()
        .parse(expression)
        .map_err(|err| format!("invalid cron expression `{expression}`: {err}"))
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
    #[serde(alias = "watchdog_id")]
    schedule_id: Option<String>,
    status: WorkflowQueuedPromptStatus,
    created_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dispatched_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workflow_run_id: Option<String>,
}

pub(crate) struct WorkflowQueuedPromptInput {
    pub(crate) id: String,
    pub(crate) queue_id: String,
    pub(crate) workflow_id: String,
    pub(crate) endpoint_id: String,
    pub(crate) prompt: Option<String>,
    pub(crate) publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
    pub(crate) source: WorkflowQueuedPromptSource,
    pub(crate) schedule_id: Option<String>,
}

impl WorkflowQueuedPrompt {
    pub(crate) fn new(input: WorkflowQueuedPromptInput) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: input.id,
            queue_id: input.queue_id,
            workflow_id: input.workflow_id,
            endpoint_id: input.endpoint_id,
            prompt: input.prompt,
            publication_invocation: input.publication_invocation,
            source: input.source,
            schedule_id: input.schedule_id,
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
    pub fn schedule_id(&self) -> Option<&str> {
        self.schedule_id.as_deref()
    }
    pub fn watchdog_id(&self) -> Option<&str> {
        self.schedule_id()
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

    pub(crate) fn mark_queued_for_retry(&mut self) {
        self.status = WorkflowQueuedPromptStatus::Queued;
        self.updated_at_ms = unix_epoch_ms();
        self.dispatched_at_ms = None;
        self.workflow_run_id = None;
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

pub(crate) struct WorkflowScheduleReconfiguration {
    pub(crate) endpoint_id: String,
    pub(crate) queue_id: Option<String>,
    pub(crate) trigger: WorkflowScheduleTrigger,
    pub(crate) invocation_prompt: String,
    pub(crate) overlap_policy: WorkflowScheduleOverlapPolicy,
    pub(crate) max_runs: Option<u64>,
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowScheduleDefinition {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_id: Option<String>,
    enabled: bool,
    trigger: WorkflowScheduleTrigger,
    invocation_prompt: String,
    overlap_policy: WorkflowScheduleOverlapPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_runs: Option<u64>,
    runs_started: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_scheduled_for_ms: Option<u64>,
    next_run_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_workflow_run_id: Option<String>,
    pending_run: bool,
    created_at_ms: u64,
    updated_at_ms: u64,
}

fn default_workflow_schedule_trigger() -> WorkflowScheduleTrigger {
    WorkflowScheduleTrigger::interval(60)
}

#[derive(Deserialize)]
struct WorkflowScheduleDefinitionWire {
    id: String,
    workflow_id: String,
    endpoint_id: String,
    #[serde(default)]
    queue_id: Option<String>,
    enabled: bool,
    #[serde(default)]
    trigger: Option<WorkflowScheduleTrigger>,
    #[serde(default)]
    interval_seconds: Option<u64>,
    invocation_prompt: String,
    #[serde(default)]
    overlap_policy: Option<WorkflowScheduleOverlapPolicy>,
    #[serde(default)]
    policy: Option<WorkflowScheduleOverlapPolicy>,
    #[serde(default)]
    max_runs: Option<u64>,
    #[serde(default)]
    max_wakeups: Option<u64>,
    #[serde(default)]
    runs_started: Option<u64>,
    #[serde(default)]
    wakeups_executed: Option<u64>,
    #[serde(default)]
    last_scheduled_for_ms: Option<u64>,
    next_run_at_ms: u64,
    #[serde(default)]
    last_run_at_ms: Option<u64>,
    #[serde(default)]
    last_status: Option<String>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    last_workflow_run_id: Option<String>,
    #[serde(default)]
    pending_run: bool,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl<'de> Deserialize<'de> for WorkflowScheduleDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WorkflowScheduleDefinitionWire::deserialize(deserializer)?;
        let trigger = wire
            .trigger
            .or_else(|| wire.interval_seconds.map(WorkflowScheduleTrigger::interval))
            .unwrap_or_else(default_workflow_schedule_trigger);
        let overlap_policy = wire
            .overlap_policy
            .or(wire.policy)
            .ok_or_else(|| de::Error::missing_field("overlap_policy"))?;
        Ok(Self {
            id: wire.id,
            workflow_id: wire.workflow_id,
            endpoint_id: wire.endpoint_id,
            queue_id: wire.queue_id,
            enabled: wire.enabled,
            trigger,
            invocation_prompt: wire.invocation_prompt,
            overlap_policy,
            max_runs: wire.max_runs.or(wire.max_wakeups),
            runs_started: wire.runs_started.or(wire.wakeups_executed).unwrap_or(0),
            last_scheduled_for_ms: wire.last_scheduled_for_ms,
            next_run_at_ms: wire.next_run_at_ms,
            last_run_at_ms: wire.last_run_at_ms,
            last_status: wire.last_status,
            last_error: wire.last_error,
            last_workflow_run_id: wire.last_workflow_run_id,
            pending_run: wire.pending_run,
            created_at_ms: wire.created_at_ms,
            updated_at_ms: wire.updated_at_ms,
        })
    }
}

impl WorkflowScheduleDefinition {
    pub fn new(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        interval_seconds: u64,
        invocation_prompt: impl Into<String>,
        policy: WorkflowWatchdogPolicy,
        max_wakeups: Option<u64>,
    ) -> Self {
        Self::new_with_trigger(
            id,
            workflow_id,
            endpoint_id,
            WorkflowScheduleTrigger::interval(interval_seconds),
            invocation_prompt,
            policy,
            max_wakeups,
        )
    }

    pub fn new_with_trigger(
        id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        trigger: WorkflowScheduleTrigger,
        invocation_prompt: impl Into<String>,
        overlap_policy: WorkflowScheduleOverlapPolicy,
        max_runs: Option<u64>,
    ) -> Self {
        let now = unix_epoch_ms();
        let next_run_at_ms = trigger
            .next_run_after_ms(now)
            .unwrap_or_else(|_| now.saturating_add(60_000));
        Self {
            id: id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            queue_id: None,
            enabled: true,
            trigger,
            invocation_prompt: invocation_prompt.into(),
            overlap_policy,
            max_runs,
            runs_started: 0,
            last_scheduled_for_ms: None,
            next_run_at_ms,
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
        match &self.trigger {
            WorkflowScheduleTrigger::Interval { every_seconds } => *every_seconds,
            WorkflowScheduleTrigger::Cron { .. } => 60,
        }
    }
    pub fn trigger(&self) -> &WorkflowScheduleTrigger {
        &self.trigger
    }
    pub fn invocation_prompt(&self) -> &str {
        &self.invocation_prompt
    }
    pub fn policy(&self) -> WorkflowWatchdogPolicy {
        self.overlap_policy
    }
    pub fn overlap_policy(&self) -> WorkflowScheduleOverlapPolicy {
        self.overlap_policy
    }
    pub fn max_wakeups(&self) -> Option<u64> {
        self.max_runs
    }
    pub fn max_runs(&self) -> Option<u64> {
        self.max_runs
    }
    pub fn wakeups_executed(&self) -> u64 {
        self.runs_started
    }
    pub fn runs_started(&self) -> u64 {
        self.runs_started
    }
    pub fn last_scheduled_for_ms(&self) -> Option<u64> {
        self.last_scheduled_for_ms
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

    pub(crate) fn reconfigure(&mut self, replacement: WorkflowScheduleReconfiguration) {
        let now = unix_epoch_ms();
        self.endpoint_id = replacement.endpoint_id;
        self.queue_id = replacement.queue_id;
        self.trigger = replacement.trigger;
        self.invocation_prompt = replacement.invocation_prompt;
        self.overlap_policy = replacement.overlap_policy;
        self.max_runs = replacement.max_runs;
        self.enabled = replacement.enabled;
        self.next_run_at_ms = self
            .trigger
            .next_run_after_ms(now)
            .unwrap_or_else(|_| now.saturating_add(60_000));
        self.updated_at_ms = now;
    }

    pub fn set_next_run_at_ms(&mut self, value: u64) {
        self.next_run_at_ms = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn schedule_next_run_after_ms(&mut self, after_ms: u64) -> Result<u64, String> {
        let next_run_at_ms = self.trigger.next_run_after_ms(after_ms)?;
        self.last_scheduled_for_ms = Some(next_run_at_ms);
        self.set_next_run_at_ms(next_run_at_ms);
        Ok(next_run_at_ms)
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
        self.max_runs = value;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_wakeups_executed(&mut self, value: u64) {
        self.runs_started = value;
        self.updated_at_ms = unix_epoch_ms();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn cron_trigger_preview_preserves_seconds() {
        let trigger = WorkflowScheduleTrigger::cron("15 30 14 * * *", "Europe/Berlin");
        let start = Utc
            .with_ymd_and_hms(2026, 7, 1, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;

        let preview = trigger.preview_run_times_after_ms(start, 2).unwrap();

        assert_eq!(preview.len(), 2);
        let first = Utc
            .timestamp_millis_opt(preview[0] as i64)
            .single()
            .unwrap()
            .with_timezone(&"Europe/Berlin".parse::<Tz>().unwrap());
        assert_eq!(first.hour(), 14);
        assert_eq!(first.minute(), 30);
        assert_eq!(first.second(), 15);
        assert_eq!(first.day(), 1);
        let second = Utc
            .timestamp_millis_opt(preview[1] as i64)
            .single()
            .unwrap()
            .with_timezone(&"Europe/Berlin".parse::<Tz>().unwrap());
        assert_eq!(second.second(), 15);
        assert_eq!(second.day(), 2);
    }

    #[test]
    fn cron_trigger_rejects_missing_seconds() {
        let trigger = WorkflowScheduleTrigger::cron("30 14 * * *", "Europe/Berlin");

        assert!(trigger.validate().is_err());
    }

    #[test]
    fn cron_trigger_rejects_invalid_timezone() {
        let trigger = WorkflowScheduleTrigger::cron("0 30 14 * * *", "Berlin");

        assert!(trigger.validate().is_err());
    }

    #[test]
    fn cron_keeps_new_york_wall_clock_time_across_spring_dst() {
        let trigger = WorkflowScheduleTrigger::cron("0 0 9 * * *", "America/New_York");
        let start = Utc
            .with_ymd_and_hms(2026, 3, 7, 8, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;

        let runs = trigger.preview_run_times_after_ms(start, 2).unwrap();
        let timezone = "America/New_York".parse::<Tz>().unwrap();
        let local = runs
            .iter()
            .map(|run| {
                Utc.timestamp_millis_opt(*run as i64)
                    .unwrap()
                    .with_timezone(&timezone)
            })
            .collect::<Vec<_>>();

        assert_eq!(local[0].hour(), 9);
        assert_eq!(local[1].hour(), 9);
        assert_eq!(runs[1] - runs[0], 23 * 60 * 60 * 1_000);
    }

    #[test]
    fn cron_keeps_helsinki_wall_clock_time_across_autumn_dst() {
        let trigger = WorkflowScheduleTrigger::cron("0 0 9 * * *", "Europe/Helsinki");
        let start = Utc
            .with_ymd_and_hms(2026, 10, 23, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;

        let runs = trigger.preview_run_times_after_ms(start, 2).unwrap();
        let timezone = "Europe/Helsinki".parse::<Tz>().unwrap();
        let local = runs
            .iter()
            .map(|run| {
                Utc.timestamp_millis_opt(*run as i64)
                    .unwrap()
                    .with_timezone(&timezone)
            })
            .collect::<Vec<_>>();

        assert_eq!(local[0].hour(), 9);
        assert_eq!(local[1].hour(), 9);
        assert_eq!(runs[1] - runs[0], 25 * 60 * 60 * 1_000);
    }

    #[test]
    fn schedule_definition_deserializes_legacy_watchdog_fields() {
        let schedule: WorkflowScheduleDefinition = serde_json::from_value(serde_json::json!({
            "id": "watchdog-1",
            "workflow_id": "workflow-1",
            "endpoint_id": "endpoint-1",
            "enabled": true,
            "interval_seconds": 300,
            "invocation_prompt": "Run checks",
            "policy": "queue",
            "max_wakeups": 7,
            "wakeups_executed": 3,
            "next_run_at_ms": 10,
            "created_at_ms": 1,
            "updated_at_ms": 2
        }))
        .unwrap();

        assert_eq!(schedule.trigger(), &WorkflowScheduleTrigger::interval(300));
        assert_eq!(
            schedule.overlap_policy(),
            WorkflowScheduleOverlapPolicy::Queue
        );
        assert_eq!(schedule.max_runs(), Some(7));
        assert_eq!(schedule.runs_started(), 3);
    }
}

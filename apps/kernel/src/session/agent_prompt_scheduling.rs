use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPromptScheduleKind {
    Once,
    Recurring,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPromptSchedule {
    id: String,
    agent_id: String,
    kind: AgentPromptScheduleKind,
    interval_seconds: u64,
    prompt: String,
    created_at_ms: u64,
    next_run_at_ms: u64,
    #[serde(default)]
    runs_dispatched: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_triggered_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip)]
    dispatch_in_flight: bool,
}

impl AgentPromptSchedule {
    pub fn new(
        id: impl Into<String>,
        agent_id: impl Into<String>,
        kind: AgentPromptScheduleKind,
        interval_seconds: u64,
        prompt: impl Into<String>,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            agent_id: agent_id.into(),
            kind,
            interval_seconds,
            prompt: prompt.into(),
            created_at_ms,
            next_run_at_ms: created_at_ms.saturating_add(interval_seconds.saturating_mul(1_000)),
            runs_dispatched: 0,
            last_triggered_at_ms: None,
            last_error: None,
            dispatch_in_flight: false,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn kind(&self) -> AgentPromptScheduleKind {
        self.kind
    }

    pub fn interval_seconds(&self) -> u64 {
        self.interval_seconds
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn next_run_at_ms(&self) -> u64 {
        self.next_run_at_ms
    }

    pub fn runs_dispatched(&self) -> u64 {
        self.runs_dispatched
    }

    pub fn last_triggered_at_ms(&self) -> Option<u64> {
        self.last_triggered_at_ms
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn dispatch_in_flight(&self) -> bool {
        self.dispatch_in_flight
    }

    pub(crate) fn claim_dispatch(&mut self, now_ms: u64) -> bool {
        if self.dispatch_in_flight || now_ms < self.next_run_at_ms {
            return false;
        }
        self.dispatch_in_flight = true;
        true
    }

    pub(crate) fn mark_dispatch_succeeded(&mut self, now_ms: u64) {
        self.dispatch_in_flight = false;
        self.runs_dispatched = self.runs_dispatched.saturating_add(1);
        self.last_triggered_at_ms = Some(now_ms);
        self.last_error = None;
        if self.kind == AgentPromptScheduleKind::Recurring {
            self.next_run_at_ms =
                now_ms.saturating_add(self.interval_seconds.saturating_mul(1_000));
        }
    }

    pub(crate) fn mark_dispatch_failed(&mut self, now_ms: u64, error: String) {
        self.dispatch_in_flight = false;
        self.last_triggered_at_ms = Some(now_ms);
        self.last_error = Some(error);
        self.next_run_at_ms =
            now_ms.saturating_add(self.interval_seconds.max(1).saturating_mul(1_000));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPromptScheduleDispatch {
    pub session_id: String,
    pub schedule_id: String,
    pub agent_id: String,
    pub prompt: String,
}

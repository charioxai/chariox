//! Agent triggers fire prompts into an agent's own conversation.
//! `/wait-in` and `/wait-every` are schedule triggers. Sessions written before
//! protocol 576 stored them as `agent_prompt_schedules`; they are migrated when
//! read, and the same legacy shape is still written for older clients.
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What a firing does while its agent is running a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTriggerBusyPolicy {
    /// Wait in the agent's prompt queue.
    Queue,
    /// Join the running turn.
    Steer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTriggerRepeat {
    Once,
    Every,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentTriggerSource {
    Schedule {
        interval_seconds: u64,
        repeat: AgentTriggerRepeat,
        next_run_at_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTriggerOutcome {
    Started,
    Queued,
    Steered,
    Coalesced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTrigger {
    id: String,
    agent_id: String,
    /// Absent for `/wait` schedules created before agent triggers existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_by_user_id: Option<String>,
    source: AgentTriggerSource,
    busy_policy: AgentTriggerBusyPolicy,
    prompt: String,
    enabled: bool,
    created_at_ms: u64,
    #[serde(default)]
    fired: u64,
    #[serde(default)]
    coalesced: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_fired_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_outcome: Option<AgentTriggerOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip)]
    dispatch_in_flight: bool,
    #[serde(skip)]
    dispatch_generation: u64,
}

impl AgentTrigger {
    #[expect(
        clippy::too_many_arguments,
        reason = "Every field of a new schedule trigger is required and distinct."
    )]
    pub fn schedule(
        id: impl Into<String>,
        agent_id: impl Into<String>,
        created_by_user_id: impl Into<String>,
        interval_seconds: u64,
        repeat: AgentTriggerRepeat,
        busy_policy: AgentTriggerBusyPolicy,
        prompt: impl Into<String>,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            agent_id: agent_id.into(),
            created_by_user_id: Some(created_by_user_id.into()),
            source: AgentTriggerSource::Schedule {
                interval_seconds,
                repeat,
                next_run_at_ms: created_at_ms
                    .saturating_add(interval_seconds.saturating_mul(1_000)),
            },
            busy_policy,
            prompt: prompt.into(),
            enabled: true,
            created_at_ms,
            fired: 0,
            coalesced: 0,
            last_fired_at_ms: None,
            last_outcome: None,
            last_error: None,
            dispatch_in_flight: false,
            dispatch_generation: 0,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn created_by_user_id(&self) -> Option<&str> {
        self.created_by_user_id.as_deref()
    }

    pub fn source(&self) -> &AgentTriggerSource {
        &self.source
    }

    pub fn busy_policy(&self) -> AgentTriggerBusyPolicy {
        self.busy_policy
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn fired(&self) -> u64 {
        self.fired
    }

    pub fn coalesced(&self) -> u64 {
        self.coalesced
    }

    pub fn last_fired_at_ms(&self) -> Option<u64> {
        self.last_fired_at_ms
    }

    pub fn last_outcome(&self) -> Option<AgentTriggerOutcome> {
        self.last_outcome
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn next_run_at_ms(&self) -> u64 {
        let AgentTriggerSource::Schedule { next_run_at_ms, .. } = &self.source;
        *next_run_at_ms
    }

    pub(crate) fn is_one_shot(&self) -> bool {
        let AgentTriggerSource::Schedule { repeat, .. } = &self.source;
        *repeat == AgentTriggerRepeat::Once
    }

    pub(crate) fn dispatch_in_flight(&self) -> bool {
        self.dispatch_in_flight
    }

    pub(crate) fn claim_dispatch(&mut self, now_ms: u64) -> bool {
        if !self.enabled || self.dispatch_in_flight || now_ms < self.next_run_at_ms() {
            return false;
        }
        self.dispatch_in_flight = true;
        self.dispatch_generation = self.dispatch_generation.wrapping_add(1);
        true
    }

    pub(crate) fn dispatch_generation(&self) -> u64 {
        self.dispatch_generation
    }

    pub(crate) fn matches_dispatch_claim(&self, generation: u64) -> bool {
        self.enabled && self.dispatch_in_flight && self.dispatch_generation == generation
    }

    /// A firing reached the agent. `steer_refusal` is set when a steer policy
    /// firing had to stay queued.
    pub(crate) fn mark_fired(
        &mut self,
        now_ms: u64,
        outcome: AgentTriggerOutcome,
        steer_refusal: Option<String>,
    ) {
        self.dispatch_in_flight = false;
        self.fired = self.fired.saturating_add(1);
        self.last_fired_at_ms = Some(now_ms);
        self.last_outcome = Some(outcome);
        self.last_error = steer_refusal;
        self.schedule_next_run(now_ms, 0);
    }

    pub(crate) fn mark_coalesced(&mut self, now_ms: u64) {
        self.dispatch_in_flight = false;
        self.coalesced = self.coalesced.saturating_add(1);
        self.last_outcome = Some(AgentTriggerOutcome::Coalesced);
        self.schedule_next_run(now_ms, 0);
    }

    pub(crate) fn mark_failed(&mut self, now_ms: u64, error: String) {
        self.dispatch_in_flight = false;
        self.last_error = Some(error);
        self.schedule_next_run(now_ms, 1);
    }

    pub(crate) fn update(
        &mut self,
        busy_policy: Option<AgentTriggerBusyPolicy>,
        prompt: Option<String>,
        enabled: Option<bool>,
        now_ms: u64,
    ) {
        if let Some(busy_policy) = busy_policy {
            self.busy_policy = busy_policy;
        }
        if let Some(prompt) = prompt {
            self.prompt = prompt;
        }
        if let Some(enabled) = enabled {
            // A re-enabled schedule counts a full interval from now instead of
            // firing at once for the time it was off.
            if enabled && !self.enabled {
                self.schedule_next_run(now_ms, 0);
            }
            if !enabled {
                self.dispatch_in_flight = false;
                self.dispatch_generation = self.dispatch_generation.wrapping_add(1);
            }
            self.enabled = enabled;
        }
    }

    fn schedule_next_run(&mut self, now_ms: u64, minimum_interval_seconds: u64) {
        let AgentTriggerSource::Schedule {
            interval_seconds,
            next_run_at_ms,
            ..
        } = &mut self.source;
        *next_run_at_ms = now_ms.saturating_add(
            (*interval_seconds)
                .max(minimum_interval_seconds)
                .saturating_mul(1_000),
        );
    }

    fn from_legacy_schedule(schedule: AgentPromptSchedule) -> Self {
        Self {
            id: schedule.id,
            agent_id: schedule.agent_id,
            created_by_user_id: None,
            source: AgentTriggerSource::Schedule {
                interval_seconds: schedule.interval_seconds,
                repeat: match schedule.kind {
                    AgentPromptScheduleKind::Once => AgentTriggerRepeat::Once,
                    AgentPromptScheduleKind::Recurring => AgentTriggerRepeat::Every,
                },
                next_run_at_ms: schedule.next_run_at_ms,
            },
            busy_policy: AgentTriggerBusyPolicy::Queue,
            prompt: schedule.prompt,
            enabled: true,
            created_at_ms: schedule.created_at_ms,
            fired: schedule.runs_dispatched,
            coalesced: 0,
            last_fired_at_ms: schedule.last_triggered_at_ms,
            last_outcome: None,
            last_error: schedule.last_error,
            dispatch_in_flight: false,
            dispatch_generation: 0,
        }
    }

    /// The protocol < 576 view of an enabled schedule trigger.
    pub fn legacy_schedule(&self) -> Option<AgentPromptSchedule> {
        if !self.enabled {
            return None;
        }
        let AgentTriggerSource::Schedule {
            interval_seconds,
            repeat,
            next_run_at_ms,
        } = &self.source;
        Some(AgentPromptSchedule {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            kind: match repeat {
                AgentTriggerRepeat::Once => AgentPromptScheduleKind::Once,
                AgentTriggerRepeat::Every => AgentPromptScheduleKind::Recurring,
            },
            interval_seconds: *interval_seconds,
            prompt: self.prompt.clone(),
            created_at_ms: self.created_at_ms,
            next_run_at_ms: *next_run_at_ms,
            runs_dispatched: self.fired,
            last_triggered_at_ms: self.last_fired_at_ms,
            last_error: self.last_error.clone(),
        })
    }
}

/// The `/wait-in` and `/wait-every` kinds of protocol < 576 clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPromptScheduleKind {
    Once,
    Recurring,
}

/// The protocol < 576 schedule shape: read to migrate, written for older clients.
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
}

impl AgentPromptSchedule {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn next_run_at_ms(&self) -> u64 {
        self.next_run_at_ms
    }
}

/// A session's triggers. Serialized as `agent_triggers` plus the derived
/// legacy `agent_prompt_schedules`; reading migrates legacy entries that have
/// no trigger yet, so a session written by an older kernel loses none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentTriggers(pub(crate) Vec<AgentTrigger>);

impl Serialize for AgentTriggers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let legacy = self
            .0
            .iter()
            .filter_map(AgentTrigger::legacy_schedule)
            .collect::<Vec<_>>();
        let mut map = serializer.serialize_map(None)?;
        if !self.0.is_empty() {
            map.serialize_entry("agent_triggers", &self.0)?;
        }
        if !legacy.is_empty() {
            map.serialize_entry("agent_prompt_schedules", &legacy)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AgentTriggers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Record {
            #[serde(default)]
            agent_triggers: Vec<AgentTrigger>,
            #[serde(default)]
            agent_prompt_schedules: Vec<AgentPromptSchedule>,
        }
        let Record {
            mut agent_triggers,
            agent_prompt_schedules,
        } = Record::deserialize(deserializer)?;
        for schedule in agent_prompt_schedules {
            if !agent_triggers
                .iter()
                .any(|trigger| trigger.id == schedule.id)
            {
                agent_triggers.push(AgentTrigger::from_legacy_schedule(schedule));
            }
        }
        Ok(Self(agent_triggers))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTriggerDispatch {
    pub(crate) claim_generation: u64,
    pub session_id: String,
    pub trigger_id: String,
    pub agent_id: String,
    pub prompt: String,
    pub busy_policy: AgentTriggerBusyPolicy,
}

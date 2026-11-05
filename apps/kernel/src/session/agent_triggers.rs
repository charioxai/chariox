//! Agent triggers fire prompts into an agent's own conversation.
//! `/wait-in` and `/wait-every` are schedule triggers.
use serde::{Deserialize, Serialize};

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
    Http {
        ingress_id: String,
        port: std::num::NonZeroU16,
    },
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

/// What the pane shows next to a trigger firing it could not confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerDeliveryState {
    /// The provider may or may not hold the message; it is never sent again by itself.
    NotConfirmed,
    /// A person sent the firing once more.
    Resent,
}

/// A firing whose delivery could not be confirmed, kept so the person can
/// resend it. Every firing is already in the transcript as a labelled prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDelivery {
    prompt_id: String,
    trigger_id: String,
    agent_id: String,
    origin: super::PromptOrigin,
    prompt: String,
    busy_policy: AgentTriggerBusyPolicy,
    state: TriggerDeliveryState,
    recorded_at_ms: u64,
}

impl TriggerDelivery {
    pub(crate) fn not_confirmed(
        prompt_id: &str,
        trigger_id: &str,
        agent_id: &str,
        origin: super::PromptOrigin,
        prompt: &str,
        busy_policy: AgentTriggerBusyPolicy,
        recorded_at_ms: u64,
    ) -> Self {
        Self {
            prompt_id: prompt_id.to_string(),
            trigger_id: trigger_id.to_string(),
            agent_id: agent_id.to_string(),
            origin,
            prompt: prompt.to_string(),
            busy_policy,
            state: TriggerDeliveryState::NotConfirmed,
            recorded_at_ms,
        }
    }

    pub fn prompt_id(&self) -> &str {
        &self.prompt_id
    }

    pub fn trigger_id(&self) -> &str {
        &self.trigger_id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn origin(&self) -> super::PromptOrigin {
        self.origin
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn busy_policy(&self) -> AgentTriggerBusyPolicy {
        self.busy_policy
    }

    pub fn state(&self) -> TriggerDeliveryState {
        self.state
    }

    pub(crate) fn set_state(&mut self, state: TriggerDeliveryState) {
        self.state = state;
    }
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

    pub(crate) fn http(
        id: String,
        agent_id: &str,
        owner: &str,
        port: std::num::NonZeroU16,
        policy: AgentTriggerBusyPolicy,
        prompt: String,
        now_ms: u64,
    ) -> Self {
        Self {
            id: id.clone(),
            agent_id: agent_id.into(),
            created_by_user_id: Some(owner.into()),
            source: AgentTriggerSource::Http {
                ingress_id: id,
                port,
            },
            busy_policy: policy,
            prompt,
            enabled: true,
            created_at_ms: now_ms,
            fired: 0,
            coalesced: 0,
            last_fired_at_ms: None,
            last_outcome: None,
            last_error: None,
            dispatch_in_flight: false,
            dispatch_generation: 0,
        }
    }

    pub(crate) fn claim_http_dispatch(&mut self) -> bool {
        if !matches!(self.source, AgentTriggerSource::Http { .. })
            || !self.enabled
            || self.dispatch_in_flight
        {
            return false;
        }
        self.dispatch_in_flight = true;
        self.dispatch_generation = self.dispatch_generation.wrapping_add(1);
        true
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub(crate) fn enroll_local_owner(&mut self, owner: &str) {
        if self.created_by_user_id.as_deref() == Some(super::DEFAULT_LOCAL_USER_ID) {
            self.created_by_user_id = Some(owner.into());
        }
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
        match &self.source {
            AgentTriggerSource::Schedule { next_run_at_ms, .. } => *next_run_at_ms,
            AgentTriggerSource::Http { .. } => u64::MAX,
        }
    }

    pub(crate) fn is_one_shot(&self) -> bool {
        matches!(
            self.source,
            AgentTriggerSource::Schedule {
                repeat: AgentTriggerRepeat::Once,
                ..
            }
        )
    }

    pub(crate) fn dispatch_in_flight(&self) -> bool {
        self.dispatch_in_flight
    }

    pub(crate) fn claim_dispatch(&mut self, now_ms: u64) -> bool {
        if !matches!(self.source, AgentTriggerSource::Schedule { .. })
            || !self.enabled
            || self.dispatch_in_flight
            || now_ms < self.next_run_at_ms()
        {
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

    pub(crate) fn defer_dispatch(&mut self, now_ms: u64) {
        self.dispatch_in_flight = false;
        self.schedule_next_run(now_ms, 1);
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
        } = &mut self.source
        else {
            return;
        };
        *next_run_at_ms = now_ms.saturating_add(
            (*interval_seconds)
                .max(minimum_interval_seconds)
                .saturating_mul(1_000),
        );
    }
}

/// A session's triggers, serialized as `agent_triggers`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct AgentTriggers(pub(crate) Vec<AgentTrigger>);

impl AgentTriggers {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTriggerDispatch {
    pub(crate) payload: Option<serde_json::Value>,
    pub(crate) claim_generation: u64,
    pub session_id: String,
    pub trigger_id: String,
    pub agent_id: String,
    pub prompt: String,
    pub busy_policy: AgentTriggerBusyPolicy,
}

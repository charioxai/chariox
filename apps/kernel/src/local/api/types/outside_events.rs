//! MP-08 / MP-09 / MP-10 / MP-11: pulled events for principals outside Rooms.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventTarget {
    pub kernel: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeEventsRequest {
    pub targets: Vec<EventTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsubscribeEventsRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<EventTarget>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetEventSubscriptionRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEventsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    #[serde(default = "default_event_limit")]
    pub limit: usize,
}
fn default_event_limit() -> usize {
    1000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitEventsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    pub timeout_ms: u64,
    #[serde(default = "default_event_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AckEventsRequest {
    pub cursor: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OutsideEventValue {
    Agent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_state: Option<EventRunState>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_state: Option<crate::durable_state::agent_lifecycle::ExecutionState>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Present on an `error` run state: the typed cause of the failed turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<crate::provider::ProviderFailure>,
    },
    Interaction {
        state: EventInteractionState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pointer: Option<EventApprovalPointer>,
    },
    Terminal {
        exit_code: Option<u32>,
    },
    /// The agent's provider took the steer; `steer_id` is the id `SteerActivePrompt` returned.
    Steer {
        steer_id: String,
    },
    Session {
        state: EventSessionState,
    },
    Gap {
        kernel: String,
        dropped: u64,
        from: u64,
        to: u64,
    },
    SubscriptionEnded {
        reason: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventInteractionState {
    Pending,
    Resolved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSessionState {
    AgentAdded,
    AgentRemoved,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutsideEvent {
    pub cursor: u64,
    pub kernel: String,
    pub machine: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(flatten)]
    pub value: OutsideEventValue,
    pub summary: String,
    pub at_ms: u64,
    pub refs: EventRefs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventKernelHealth {
    pub kernel: String,
    pub online: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<EventKernelIssue>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventKernelIssue {
    pub code: String,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSubscription {
    pub subscription_id: String,
    pub targets: Vec<EventTarget>,
    pub kernels: Vec<EventKernelHealth>,
    pub acknowledged_cursor: u64,
    pub ended: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsBatch {
    pub events: Vec<OutsideEvent>,
    pub next_cursor: u64,
    pub oldest_cursor: u64,
    pub timed_out: bool,
}

/// The section 8 pointer has no prompt body or passkey material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventApprovalPointer {
    pub approval_id: String,
    pub kind: String,
    pub kernel: String,
    pub one_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRunState {
    Idle,
    Working,
    Error,
}

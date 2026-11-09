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
    /// Notification sources of this kernel (workflow exits and owner actions) by id. Their
    /// occurrences reach the grant's own event stream; nothing else can receive them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsubscribeEventsRequest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<EventTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetEventSubscriptionRequest {}

/// The notification sources a grant may follow with `SubscribeEvents{sources}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListNotificationSourcesRequest {}

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
        /// The turn this run state ends, with how it ended: present on `idle` and
        /// `error` once the agent has settled a turn.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<crate::provider::TurnOutcome>,
    },
    Interaction {
        state: EventInteractionState,
        /// A decision only the owner's terminal may answer: where to answer it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pointer: Option<EventApprovalPointer>,
        /// A pending decision the holder may answer, in the form to show the person.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        question: Option<Box<EventQuestion>>,
        /// How a resolved decision ended.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution: Option<EventResolution>,
        /// A sub-agent's decision the user holds is shown in this agent's panel: the room agent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shown_in: Option<String>,
    },
    Terminal {
        exit_code: Option<u32>,
    },
    /// The agent's provider took the steer; `steer_id` is the id `SteerActivePrompt` returned.
    Steer {
        steer_id: String,
    },
    /// A command waits for room or runs again: `state` is the row
    /// `session_processes_changed` shows for `process_id` at that moment.
    Process {
        process_id: String,
        state: crate::runtime::process_resources::SessionProcessState,
        /// How long it waited, once it runs again.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        waited_ms: Option<u64>,
        /// Why the pool ended the command, when it did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ended: Option<String>,
    },
    Session {
        state: EventSessionState,
    },
    /// A sub-agent tree below `root` changed: read `ListDescendants` again.
    /// `rev` is the revision that read returns once it includes the change.
    AgentTree {
        root: String,
        rev: u64,
    },
    Gap {
        kernel: String,
        dropped: u64,
        from: u64,
        to: u64,
    },
    /// Gateway observation of a followed kernel; `reason` is a public error code
    /// or `connected`, never a remote error message.
    KernelHealth {
        online: bool,
        reason: String,
    },
    SubscriptionEnded {
        reason: String,
    },
    /// One occurrence of a notification source the grant follows. Delivered once per
    /// occurrence until acknowledged; `deadline_ms` is when an unread one expires.
    Notification {
        source_id: String,
        source_kind: NotificationSourceKind,
        display_name: String,
        occurrence_id: String,
        output: crate::session::WorkflowOutputPayload,
        deadline_ms: u64,
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
/// A notification source the grant follows. `available` turns false when the source can no
/// longer emit (disabled, workflow or Room gone); the follow stays until the grant drops it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSourceFollow {
    pub source_id: String,
    pub display_name: String,
    pub kind: NotificationSourceKind,
    pub session_id: String,
    pub subscription_id: String,
    pub available: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSubscription {
    pub subscription_id: String,
    pub targets: Vec<EventTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<EventSourceFollow>,
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

/// The decision as the person sees it. It carries only what a holder may answer:
/// no choice reply texts, passkey flags or secret inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQuestion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub message: String,
    pub choices: Vec<EventQuestionChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_reply: Option<EventQuestionTextReply>,
    /// When the kernel applies `default_on_timeout`, if the decision expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_on_timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQuestionChoice {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<crate::session::RuntimeInteractionChoiceStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQuestionTextReply {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub min_length: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

/// Which surface closed a decision. `choice_id` is the answer applied, never a text reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventResolution {
    pub via: EventResolutionVia,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventResolutionVia {
    Terminal,
    ExternalAgent,
    /// A parent agent answered its sub-agent's decision.
    ParentAgent,
    Timeout,
    /// Withdrawn without an answer: the asker went away or a newer decision replaced it.
    Cancelled,
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

use serde::{Deserialize, Deserializer, Serialize};

use crate::session::{
    AgentHttpMethod, AgentHttpOptionsRequest, AgentHttpParser, AgentHttpResponseMode,
    AgentTriggerBusyPolicy, AgentTriggerNotificationSource, AgentTriggerRepeat,
    WorkflowScheduleOverlapPolicy, WorkflowScheduleTrigger,
};

/// Protocol 1007: what fires a new agent trigger. HTTP and schedule take the Workflows
/// panel's trigger options; anything left out takes the publication or schedule default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentTriggerSourceRequest {
    Http {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<AgentHttpMethod>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parser: Option<AgentHttpParser>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<AgentHttpResponseMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sync_timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poll_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_schema: Option<serde_json::Value>,
    },
    Schedule {
        trigger: WorkflowScheduleTrigger,
        repeat: AgentTriggerRepeat,
        /// Omitted means skip, as `/wait-every` always did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        overlap_policy: Option<WorkflowScheduleOverlapPolicy>,
        /// Omitted means the workflow schedule default, `null` never stops, N stops after N firings.
        #[serde(
            default,
            deserialize_with = "present_value",
            skip_serializing_if = "Option::is_none"
        )]
        max_runs: Option<Option<u64>>,
    },
    /// Subscribes the agent to a workflow exit or an App event; every occurrence is queued.
    Notification {
        source: AgentTriggerNotificationSource,
    },
}

impl AgentTriggerSourceRequest {
    /// An HTTP trigger with the publication defaults: `/`, GET and POST, JSON body, async.
    pub fn http() -> Self {
        Self::from_http_options(AgentHttpOptionsRequest::default())
    }

    pub fn from_http_options(options: AgentHttpOptionsRequest) -> Self {
        Self::Http {
            route: options.route,
            methods: options.methods,
            parser: options.parser,
            mode: options.mode,
            sync_timeout_ms: options.sync_timeout_ms,
            poll_ms: options.poll_ms,
            input_schema: options.input_schema,
        }
    }

    /// The `/wait-in` and `/wait-every` schedule: a fixed interval, skipped while busy, never done.
    pub fn interval(every_seconds: u64, repeat: AgentTriggerRepeat) -> Self {
        Self::Schedule {
            trigger: WorkflowScheduleTrigger::interval(every_seconds),
            repeat,
            overlap_policy: None,
            max_runs: (repeat == AgentTriggerRepeat::Every).then_some(None),
        }
    }
}

/// Tells a field sent as `null` from one left out.
fn present_value<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<u64>>, D::Error> {
    Option::<u64>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAgentTriggerRequest {
    pub session_id: String,
    pub agent_id: String,
    pub source: AgentTriggerSourceRequest,
    /// What a firing does while the agent runs a turn; omitted means queue. A notification
    /// trigger always queues and refuses a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_policy: Option<AgentTriggerBusyPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// Changes only the fields that are present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAgentTriggerRequest {
    pub session_id: String,
    pub trigger_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_policy: Option<AgentTriggerBusyPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteAgentTriggerRequest {
    pub session_id: String,
    pub trigger_id: String,
}

/// Protocol 901: sends a trigger firing the provider did not confirm once
/// more, following the trigger's busy policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResendTriggerDeliveryRequest {
    pub session_id: String,
    pub prompt_id: String,
}

/// MP-08/MP-10/MP-11: external input is data; authority remains the local owner.
/// `input` is what the trigger's parser made of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeAgentTriggerRequest {
    pub session_id: String,
    pub trigger_id: String,
    pub input: serde_json::Value,
    /// The request body as it arrived, so a webhook signature can still be checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<AgentTriggerHttpBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTriggerHttpBody {
    pub body_base64: String,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentTriggerInvocation {
    Accepted {
        prompt_id: String,
        outcome: crate::session::AgentTriggerOutcome,
    },
    Offline,
    Disabled,
    Busy,
    PayloadTooLarge,
    /// The input does not match the trigger's input schema; nothing was delivered.
    InvalidInput {
        message: String,
    },
}

/// Protocol 1007: what an HTTP gateway needs to route and parse a request, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadAgentTriggerHttpOptionsRequest {
    pub session_id: String,
    pub trigger_id: String,
}

/// Protocol 1007: the reply of one HTTP firing, for a sync response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadAgentTriggerReplyRequest {
    pub session_id: String,
    pub trigger_id: String,
    pub prompt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentTriggerReply {
    /// The agent has not finished the firing's turn.
    Pending,
    Completed {
        text: String,
    },
    /// The turn ended without an answer: stopped or failed.
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAgentTriggerTranscriptRequest {
    pub session_id: String,
    pub trigger_id: String,
    #[serde(default)]
    pub cursor: Option<crate::local::SessionHistoryOutlineCursor>,
}

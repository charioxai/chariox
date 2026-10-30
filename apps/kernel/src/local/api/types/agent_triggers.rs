use serde::{Deserialize, Serialize};

use crate::session::{AgentTriggerBusyPolicy, AgentTriggerRepeat};

/// Protocol 900: what fires a new agent trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentTriggerSourceRequest {
    Schedule {
        interval_seconds: u64,
        repeat: AgentTriggerRepeat,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentTriggerRequest {
    pub session_id: String,
    pub agent_id: String,
    pub source: AgentTriggerSourceRequest,
    pub busy_policy: AgentTriggerBusyPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// Changes only the fields that are present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

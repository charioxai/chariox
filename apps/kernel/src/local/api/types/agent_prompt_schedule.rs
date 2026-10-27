use serde::{Deserialize, Serialize};

use crate::session::AgentPromptScheduleKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentPromptScheduleRequest {
    pub session_id: String,
    pub agent_id: String,
    pub kind: AgentPromptScheduleKind,
    pub interval_seconds: u64,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelAgentPromptScheduleRequest {
    pub session_id: String,
    pub schedule_id: String,
}

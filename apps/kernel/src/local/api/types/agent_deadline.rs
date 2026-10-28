//! MP-08 / MP-09 / MP-10 / MP-11: unconditional executor-local agent deadline.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetAgentDeadlineRequest {
    pub session_id: String,
    pub agent_id: String,
    pub deadline_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAgentDeadlineRequest {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDeadlineState {
    Armed,
    Reached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDeadline {
    pub session_id: String,
    pub agent_id: String,
    pub deadline_ms: u64,
    pub state: AgentDeadlineState,
    pub reached_at_ms: Option<u64>,
    /// Cleanup is retried until the executor has stopped the run and settled its turn.
    pub stopped_at_ms: Option<u64>,
}

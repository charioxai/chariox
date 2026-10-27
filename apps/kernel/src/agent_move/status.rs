//! What the person asked of an agent's execution that has not settled, published on the agent so
//! every client, including one that connected after the request, shows it and cancels it.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMoveStatus {
    /// The destination asked for; None is home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<String>,
    #[serde(flatten)]
    pub stage: AgentMoveStage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentMoveStage {
    /// The agent's turn is running; the Move starts when it ends unless it is cancelled.
    WaitingForTurn,
    Moving,
    /// Stays until the person cancels the Move or asks for it again.
    Failed {
        reason: String,
    },
}

impl AgentMoveStatus {
    pub(crate) fn new(target_ref: Option<String>, stage: AgentMoveStage) -> Self {
        Self { target_ref, stage }
    }
}

use super::action::InputTarget;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputOwnership {
    pub target: InputTarget,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingInputTakeover {
    pub target: InputTarget,
    pub human_actor_id: String,
    pub blocking_action_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TakeoverOutcome {
    Granted,
    CancellationRequired { action_ids: Vec<String> },
}

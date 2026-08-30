use super::action::InputTarget;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputOwnership {
    pub target: InputTarget,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverOutcome {
    Granted,
    CancellationRequired { action_ids: Vec<String> },
}

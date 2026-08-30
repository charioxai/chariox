use super::action::InputTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputOwnership {
    pub target: InputTarget,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverOutcome {
    Granted,
    CancellationRequired { action_ids: Vec<String> },
}

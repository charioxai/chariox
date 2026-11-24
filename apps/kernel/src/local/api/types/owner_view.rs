use serde::{Deserialize, Serialize};

/// The owner terminal's side of a presentation-only exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerViewRequest {
    Status {},
    Publish {
        generation: String,
        tab_id: String,
        rows: OwnerViewRows,
    },
    End {
        agent_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerViewRows {
    pub navigator: Vec<String>,
    pub settings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerViewWatcher {
    pub agent_id: String,
    pub name: String,
}

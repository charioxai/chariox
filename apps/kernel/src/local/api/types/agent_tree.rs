//! MP-08 / MP-09 / MP-10 / MP-11 A4: what a client reads of a sub-agent tree.
//! Data only: enums, ids and numbers. It carries no holder, path, card or
//! settled-by field, and the kernel adds no error text.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDescendantsRequest {
    pub session_id: String,
    pub agent_id: String,
}

/// `waiting`: blocked on a decision held by an ancestor or the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTreeState {
    Starting,
    Running,
    Waiting,
    Idle,
    Ended,
    Error,
    Unreachable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTreeTotals {
    pub live: u32,
    pub running: u32,
    pub needs_user: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTreeNode {
    pub agent_id: String,
    pub parent_agent_id: Option<String>,
    pub kernel_id: Option<String>,
    pub alias: Option<String>,
    pub provider: String,
    pub model: Option<String>,
    pub depth: u32,
    pub state: AgentTreeState,
    pub outcome: Option<crate::provider::TurnOutcomeKind>,
    /// The user currently holds a pending decision of this node; no holder, path or card.
    pub needs_user: bool,
    pub deadline_ms: Option<u64>,
    pub created_at_ms: u64,
}

/// The descendants of `root` in creation order (stable); the client builds the
/// hierarchy from `parent_agent_id`. `rev` grows with every change of any tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTree {
    pub root: String,
    pub rev: u64,
    pub totals: AgentTreeTotals,
    pub nodes: Vec<AgentTreeNode>,
}

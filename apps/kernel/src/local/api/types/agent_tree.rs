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
    /// The mode this node really runs in, after the limit of its parent chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    /// The permission level this node's approvals really use, after the limit of its parent chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    /// The node's effective variant; omitted when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The provider account profile the node runs under; omitted when it uses the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
    /// The workspace and worktree the node works in; omitted when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    /// The kernel that runs the node when it is not this one; omitted for a local node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_kernel_id: Option<String>,
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

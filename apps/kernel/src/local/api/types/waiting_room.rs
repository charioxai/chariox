use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomLaunchTarget {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomInventorySnapshot {
    pub inventory_version: String,
    pub sessions: Vec<WaitingRoomPublicSessionSummary>,
    pub relay_status: RelayStatus,
    #[serde(default)]
    pub terminals: Vec<TerminalRecord>,
    pub launch_target: Option<WaitingRoomLaunchTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicSnapshot {
    pub schema_version: u32,
    pub inventory_version: String,
    pub generated_at_ms: u64,
    pub sessions: Vec<WaitingRoomPublicSessionSummary>,
    pub relay_status: RelayStatus,
    #[serde(default)]
    pub terminals: Vec<TerminalRecord>,
    pub launch_target: Option<WaitingRoomLaunchTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicSessionSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at_ms: Option<u64>,
    pub status: crate::session::SessionStatus,
    pub connected_cli_count: usize,
    #[serde(default)]
    pub activity: WaitingRoomSessionActivitySummary,
    #[serde(default)]
    pub agents: Vec<WaitingRoomPublicAgentSummary>,
    #[serde(default)]
    pub workflows: Vec<WaitingRoomPublicWorkflowSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomSessionActivitySummary {
    pub agent_count: usize,
    pub working_agent_count: usize,
    pub active_prompt_count: usize,
    pub queued_prompt_count: usize,
    pub error_agent_count: usize,
    #[serde(default, skip_serializing_if = "crate::session::is_zero")]
    pub unread_idle_agent_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicItemActivitySummary {
    pub working: bool,
    pub active_prompt_count: usize,
    pub queued_prompt_count: usize,
    pub error: bool,
    #[serde(default, skip_serializing_if = "crate::session::is_false")]
    pub unread_idle_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicAgentSummary {
    pub id: String,
    pub agent_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub created_at_ms: u64,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_label: Option<String>,
    #[serde(default)]
    pub extension_grants: Vec<crate::extension::ExtensionGrant>,
    #[serde(default)]
    pub activity: WaitingRoomPublicItemActivitySummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicWorkflowSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub created_at_ms: u64,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_layout: Option<WorkflowCanvasLayout>,
    #[serde(default)]
    pub activity: WaitingRoomPublicItemActivitySummary,
    #[serde(default)]
    pub nodes: Vec<WaitingRoomPublicWorkflowNodeSummary>,
    #[serde(default)]
    pub edges: Vec<WaitingRoomPublicWorkflowEdgeSummary>,
    #[serde(default)]
    pub endpoints: Vec<WaitingRoomPublicWorkflowEndpointSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicWorkflowNodeSummary {
    pub id: String,
    pub agent_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicWorkflowEdgeSummary {
    pub id: String,
    pub from_node_id: String,
    pub to_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicWorkflowEndpointSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub entry_node_id: String,
}

impl From<WaitingRoomPublicSnapshot> for WaitingRoomInventorySnapshot {
    fn from(snapshot: WaitingRoomPublicSnapshot) -> Self {
        Self {
            inventory_version: snapshot.inventory_version,
            sessions: snapshot.sessions,
            relay_status: snapshot.relay_status,
            terminals: snapshot.terminals,
            launch_target: snapshot.launch_target,
        }
    }
}

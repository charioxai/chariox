use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWaitingRoomInventoryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWaitingRoomPublicSnapshotRequest;

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
    pub structural_version: String,
    pub activity_revision: String,
    pub sessions: Vec<WaitingRoomPublicSessionSummary>,
    #[serde(default)]
    pub projects: Vec<WaitingRoomPublicProjectSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_provider_sessions: Vec<ExternalProviderSessionRecord>,
    #[serde(default)]
    pub external_provider_sessions_has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_sessions_next_cursor: Option<String>,
    pub relay_status: RelayStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_machines: Vec<RemoteMachineRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_kernels: Vec<RelayKernelPresence>,
    #[serde(default)]
    pub terminals: Vec<TerminalRecord>,
    pub launch_target: Option<WaitingRoomLaunchTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicSnapshot {
    pub schema_version: u32,
    pub inventory_version: String,
    pub structural_version: String,
    pub activity_revision: String,
    pub generated_at_ms: u64,
    pub sessions: Vec<WaitingRoomPublicSessionSummary>,
    #[serde(default)]
    pub projects: Vec<WaitingRoomPublicProjectSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_provider_sessions: Vec<ExternalProviderSessionRecord>,
    #[serde(default)]
    pub external_provider_sessions_has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_sessions_next_cursor: Option<String>,
    pub relay_status: RelayStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_machines: Vec<RemoteMachineRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_kernels: Vec<RelayKernelPresence>,
    #[serde(default)]
    pub terminals: Vec<TerminalRecord>,
    pub launch_target: Option<WaitingRoomLaunchTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicSessionSummary {
    pub id: String,
    pub project_id: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_sent_at_ms: Option<u64>,
    pub status: crate::session::SessionStatus,
    pub connected_cli_count: usize,
    #[serde(default)]
    pub joined_collaborator_count: usize,
    #[serde(default)]
    pub pending_collaboration_invite_count: usize,
    #[serde(default)]
    pub activity: WaitingRoomSessionActivitySummary,
    #[serde(default)]
    pub agents: Vec<WaitingRoomPublicAgentSummary>,
    #[serde(default)]
    pub workflows: Vec<WaitingRoomPublicWorkflowSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicProjectSummary {
    pub id: String,
    pub owner_user_id: String,
    pub workspace_id: String,
    pub name: String,
    pub kind: crate::session::RuntimeProjectKind,
    pub status: crate::session::RuntimeProjectStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at_ms: Option<u64>,
    pub session_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session_activity_at_ms: Option<u64>,
    #[serde(default)]
    pub joined_collaborator_count: usize,
    #[serde(default)]
    pub pending_collaboration_invite_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomSessionActivitySummary {
    pub agent_count: usize,
    pub working_agent_count: usize,
    pub active_prompt_count: usize,
    pub queued_prompt_count: usize,
    pub error_agent_count: usize,
    #[serde(default, skip_serializing_if = "crate::session::is_zero")]
    pub remote_agent_count: usize,
    #[serde(default, skip_serializing_if = "crate::session::is_zero")]
    pub missing_worker_provider_run_count: usize,
    #[serde(default, skip_serializing_if = "crate::session::is_zero")]
    pub home_proxy_agent_count: usize,
    #[serde(default, skip_serializing_if = "crate::session::is_zero")]
    pub remote_extension_sync_issue_count: usize,
    #[serde(default, skip_serializing_if = "crate::session::is_zero")]
    pub remote_extension_pending_revoke_count: usize,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_sent_at_ms: Option<u64>,
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
    pub runtime_placement: WaitingRoomAgentRuntimePlacement,
    #[serde(default)]
    pub extension_grants: Vec<crate::extension::ExtensionGrant>,
    #[serde(default)]
    pub activity: WaitingRoomPublicItemActivitySummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metaagent_event_counts: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomAgentRuntimePlacement {
    pub kernel_id: String,
    pub machine_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_display_endpoint: Option<crate::slice::SliceDisplayEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicWorkflowSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wait_for_all_inputs: bool,
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
            structural_version: snapshot.structural_version,
            activity_revision: snapshot.activity_revision,
            sessions: snapshot.sessions,
            projects: snapshot.projects,
            external_provider_sessions: snapshot.external_provider_sessions,
            external_provider_sessions_has_more: snapshot.external_provider_sessions_has_more,
            external_provider_sessions_next_cursor: snapshot.external_provider_sessions_next_cursor,
            relay_status: snapshot.relay_status,
            remote_machines: snapshot.remote_machines,
            remote_kernels: snapshot.remote_kernels,
            terminals: snapshot.terminals,
            launch_target: snapshot.launch_target,
        }
    }
}

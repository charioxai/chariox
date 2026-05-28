use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachToSessionRequest {
    pub session_id: String,
    pub client_id: String,
    pub capability_level: ClientCapabilityLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachFromSessionRequest {
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessionMembersRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionInviteRequest {
    pub session_id: String,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default)]
    pub max_uses: Option<u32>,
    #[serde(default)]
    pub collaboration_level: crate::session::CollaborationLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinSessionInviteRequest {
    pub invite_token: String,
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeSessionInviteRequest {
    pub session_id: String,
    pub invite_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkspaceLinkRequest {
    pub session_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkspaceLinksRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowWorkspaceLinkRequest {
    pub session_id: String,
    pub link_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachWorkspaceLinkRequest {
    pub session_id: String,
    pub link_ref: String,
    #[serde(default)]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub repo_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachWorkspaceLinkRequest {
    pub session_id: String,
    pub link_ref: String,
    #[serde(default)]
    pub repo_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkspaceLiveSyncStatusRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLiveSyncFooterState {
    Off,
    Managed,
    Tracked,
    Syncing,
    Conflict,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLiveSyncTargetState {
    Ready,
    Degraded,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncTargetStatus {
    pub link_id: String,
    pub link_name: String,
    pub user_id: String,
    pub machine_id: String,
    pub kernel_id: String,
    pub repo_root: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub repo_fingerprint: Option<String>,
    pub status: WorkspaceLiveSyncTargetState,
    pub attached_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncConflictSummary {
    pub conflict_id: String,
    pub link_id: String,
    pub source_agent_id: String,
    pub target_user_id: String,
    pub target_repo_root: String,
    pub path: String,
    pub next_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncIgnoreStatus {
    #[serde(default)]
    pub ignore_file: Option<String>,
    #[serde(default)]
    pub rules: Vec<String>,
    pub force_excludes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceLiveSyncStatus {
    pub session_id: String,
    pub mode: crate::config::WorkspaceLiveSyncMode,
    pub footer_state: WorkspaceLiveSyncFooterState,
    pub targets: Vec<WorkspaceLiveSyncTargetStatus>,
    pub conflicts: Vec<WorkspaceLiveSyncConflictSummary>,
    pub ignore: WorkspaceLiveSyncIgnoreStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInviteRecord {
    pub invite: SessionInvite,
    pub invite_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionStateRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessionsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveSessionRequest {
    pub session_ref: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasSessionRequest {
    pub session_id: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndSessionRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteSessionRequest {
    pub session_ref: String,
    pub workspace_id: Option<String>,
}

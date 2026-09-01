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
pub struct WorkspaceLiveSyncGroupStatus {
    pub group_id: String,
    pub group_name: String,
    pub target_count: usize,
    pub ready_targets: usize,
    pub degraded_targets: usize,
    pub conflicted_targets: usize,
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
    pub sync_groups: Vec<WorkspaceLiveSyncGroupStatus>,
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
pub struct UpdateMetaagentTaskRequest {
    pub session_id: String,
    pub metaagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_markdown: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseMetaagentTaskRequest {
    pub session_id: String,
    pub metaagent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeMetaagentTaskRequest {
    pub session_id: String,
    pub metaagent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbortMetaagentTaskRequest {
    pub session_id: String,
    pub metaagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSessionsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProjectsRequest {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameProjectRequest {
    pub project_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProjectWorkspacesRequest {
    pub project_id: String,
    pub workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveProjectRequest {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProjectRequest {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreProjectRequest {
    pub project_id: String,
}

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

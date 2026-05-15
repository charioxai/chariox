use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchWorkspaceDirectoriesRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkspaceDirectoryRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkspaceWorktreesRequest {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkspaceWorktreeRequest {
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteWorkspaceWorktreeRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "crate::session::is_false")]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkspacePullRequestRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    #[serde(default, skip_serializing_if = "crate::session::is_false")]
    pub draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkspaceGitOverviewRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkspaceFilesRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkspaceFileContentRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateWorkspaceCommitMessageRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_ref: Option<String>,
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitWorkspaceChangesRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushWorkspaceBranchRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default)]
    pub force_with_lease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitAndPushWorkspaceChangesRequest {
    pub workspace_id: String,
    pub worktree_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGitCompareRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGitFileChange {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGitChangeTotals {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGitOverview {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub compare_ref: String,
    pub compare_refs: Vec<WorkspaceGitCompareRef>,
    pub totals: WorkspaceGitChangeTotals,
    pub files: Vec<WorkspaceGitFileChange>,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRepoFileEntry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRepoFileListing {
    pub workspace_id: String,
    pub worktree_id: String,
    pub path_prefix: String,
    pub compare_ref: String,
    pub total_entries: u32,
    pub truncated: bool,
    pub entries: Vec<WorkspaceRepoFileEntry>,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFileContent {
    pub workspace_id: String,
    pub worktree_id: String,
    pub path: String,
    pub name: String,
    pub language: String,
    pub mime: String,
    pub encoding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    pub size_bytes: u64,
    pub mtime_ms: u64,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub compare_ref: String,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceGitActionResult {
    pub workspace_id: String,
    pub worktree_id: String,
    pub action: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWorktreeRecord {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePullRequestRecord {
    pub workspace_id: String,
    pub worktree_id: String,
    pub branch: String,
    pub base_ref: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub draft: bool,
    pub generated_at_ms: u64,
}

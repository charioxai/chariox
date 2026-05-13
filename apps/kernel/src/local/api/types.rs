use super::*;

use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};
use arroba_relay::protocol::RelayKernelPresence;

pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachToSessionRequest {
    pub session_id: String,
    pub client_id: String,
    pub capability_level: ClientCapabilityLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchProviderRunRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub adapter_key: String,
    pub provider: String,
    pub account_profile: String,
    pub model: String,
    pub variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default)]
    pub native_tui: bool,
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
pub struct SessionInviteRecord {
    pub invite: SessionInvite,
    pub invite_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub target_agent_id: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub attachments: Vec<PromptAttachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletePromptRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelActivePromptRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSessionConfigRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub values: BTreeMap<String, String>,
    pub requires_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentConfigRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default)]
    pub clear_execution_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    #[serde(default)]
    pub clear_permission_level: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub clear_workspace_id: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub clear_worktree_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentProfileRequest {
    pub session_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub clear_effort: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSubstituteAction {
    Add {
        provider: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree_id: Option<String>,
    },
    Remove {
        index: usize,
    },
    Clear {},
    SetTimeout {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    Activate {
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Primary {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAgentSubstitutesRequest {
    pub session_id: String,
    pub agent_id: String,
    pub action: AgentSubstituteAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionStateRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDaemonHealthRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderRunRequest {
    pub provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProviderRunSelectionRequest {
    pub session_id: String,
    pub provider_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(default)]
    pub clear_variant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCatalogRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCommandCatalogsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRemoteMachinesRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRemoteMachineKernelsRequest {
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWaitingRoomInventoryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWaitingRoomPublicSnapshotRequest;

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
pub enum AgentUtilityKind {
    WorkspaceCommitMessage,
    SemanticHistorySearch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCommitMessageUtilityInput {
    pub workspace_id: String,
    pub worktree_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compare_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticHistorySearchUtilityInput {
    pub query: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workflow_id: Option<String>,
    pub machine_id: Option<String>,
    pub repo_root: Option<String>,
    pub worktree_path: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentUtilityInput {
    WorkspaceCommitMessage(WorkspaceCommitMessageUtilityInput),
    SemanticHistorySearch(SemanticHistorySearchUtilityInput),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAgentUtilityRequest {
    pub session_id: String,
    pub agent_id: String,
    pub kind: AgentUtilityKind,
    pub input: AgentUtilityInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentUtilityOutput {
    WorkspaceCommitMessage {
        message: String,
    },
    SemanticHistorySearch {
        answer: String,
        matches: Vec<SemanticHistoryMatch>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentUtilityResult {
    pub utility_run_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub kind: AgentUtilityKind,
    pub output: AgentUtilityOutput,
    pub generated_at_ms: u64,
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
pub struct ApproveRemoteMachineRequest {
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetRemoteMachineRequest {
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameRemoteMachineRequest {
    pub machine_ref: String,
    pub alias: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingInviteIntent {
    Client,
    Machine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalType {
    Cli,
    Web,
    Ios,
    Android,
}

impl TerminalType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Web => "web",
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePairingInviteRequest {
    pub intent: PairingInviteIntent,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTerminalPairingLinkRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinTerminalPairingLinkRequest {
    pub pairing_link: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinPairingInviteRequest {
    pub invite_token: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub public_key_thumbprint: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingInviteRecord {
    pub intent: PairingInviteIntent,
    pub invite_id: String,
    pub invite_token: String,
    pub relay_url: String,
    pub target_daemon_id: String,
    #[serde(default)]
    pub target_daemon_alias: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingJoinRecord {
    pub intent: PairingInviteIntent,
    pub subject_id: String,
    pub relay_url: String,
    pub target_daemon_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    pub public_key_thumbprint: String,
    pub paired_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListPairedClientsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListTerminalsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPairedClientRequest {
    pub client_id: String,
    pub public_key_thumbprint: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    #[serde(default)]
    pub paired_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokePairedClientRequest {
    pub client_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedClientRecord {
    pub client_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_type: Option<TerminalType>,
    pub public_key_thumbprint: String,
    pub paired_at_ms: u64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalRecord {
    pub terminal_id: String,
    pub terminal_type: TerminalType,
    #[serde(default)]
    pub alias: Option<String>,
    pub paired_at_ms: u64,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPairingLinkRecord {
    pub terminal_id: String,
    pub pairing_link: String,
    pub pairing_code: String,
    pub invite_id: String,
    pub relay_url: String,
    pub target_daemon_id: String,
    #[serde(default)]
    pub target_daemon_alias: Option<String>,
    pub terminal_type: TerminalType,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteMachineTrustStatus {
    Approved,
    Pending,
    Forgotten,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMachineRecord {
    pub machine_id: String,
    #[serde(default)]
    pub machine_alias: Option<String>,
    #[serde(default)]
    pub registry_alias: Option<String>,
    pub display_name: String,
    pub trust_status: RemoteMachineTrustStatus,
    pub online: bool,
    pub pending: bool,
    pub kernel_count: usize,
    #[serde(default)]
    pub available_providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigureRelayRequest {
    pub relay_url: Option<String>,
    pub relay_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayStatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartCloudRelayLoginRequest {
    pub api_url: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_alias: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub machine_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCloudRelayLoginRequest {
    pub api_url: String,
    pub device_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutCloudRelayRequest {
    #[serde(default)]
    pub revoke_client: bool,
    #[serde(default)]
    pub revoke_machine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCloudRelayClientRequest {
    pub client_id: String,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCloudRelayMachineRequest {
    pub machine_id: String,
    #[serde(default)]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectCloudRelayRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCloudRelayClientTokenRequest {
    pub target_daemon_alias: String,
    pub client_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCloudSessionInviteRequest {
    pub session_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default)]
    pub max_uses: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowCloudSessionInviteRequest {
    pub invite_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptCloudSessionInviteRequest {
    pub invite_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeCloudSessionInviteRequest {
    pub session_id: String,
    pub invite_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCloudSessionMembersRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCloudCollaboratorsRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayProfile {
    pub api_url: String,
    pub email: String,
    pub account_id: String,
    pub user_id: String,
    pub account_slug: String,
    pub realm_id: String,
    pub relay_url: String,
    pub issuer_id: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_alias: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub machine_alias: Option<String>,
    #[serde(default)]
    pub machine_credential: Option<String>,
    #[serde(default)]
    pub cloud_session_token: Option<String>,
    #[serde(default)]
    pub cloud_session_expires_at_ms: Option<u64>,
    #[serde(default)]
    pub token_expires_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayLoginStart {
    pub api_url: String,
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_at: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudRelayLoginPollStatus {
    AuthorizationPending,
    ExpiredToken,
    Approved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayLoginPoll {
    pub status: CloudRelayLoginPollStatus,
    #[serde(default)]
    pub interval_seconds: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub profile: Option<CloudRelayProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudRelayRuntimeToken {
    pub relay_url: String,
    pub relay_token: String,
    pub token_expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionInvite {
    pub invite_id: String,
    pub invite_token: String,
    pub session_id: String,
    pub account_id: String,
    pub created_by_user_id: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_uses: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionInviteDetails {
    pub invite_id: String,
    pub session_id: String,
    pub account_id: String,
    pub created_by_user_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub max_uses: Option<u32>,
    pub used_count: u32,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionInviteAcceptance {
    pub session_id: String,
    pub account_id: String,
    pub user_id: String,
    pub invited_by_user_id: String,
    pub joined_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudSessionMember {
    pub user_id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub invited_by_user_id: Option<String>,
    pub joined_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudCollaborator {
    pub user_id: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub last_collaborated_at: String,
    pub shared_session_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetUserConfigRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetUserConfigSchemaRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetUserConfigValueRequest {
    pub path: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsetUserConfigValueRequest {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfigProviderReloadSummary {
    pub reloaded: u32,
    pub deferred: u32,
    pub unaffected: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfigMutationEffect {
    pub kind: String,
    pub path: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_reload: Option<UserConfigProviderReloadSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetCredentialSecretRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteCredentialSecretRequest {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayStatus {
    pub configured: bool,
    pub connected: bool,
    pub relay_url: Option<String>,
    pub relay_token_configured: bool,
    pub daemon_id: String,
    pub machine_id: String,
    pub machine_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallMcpServerRequest {
    pub workspace_id: Option<String>,
    pub config: ArrobaMcpServerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateMcpServerRequest {
    pub workspace_id: Option<String>,
    pub config: ArrobaMcpServerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallMcpServerRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMcpServersRequest {
    pub workspace_id: Option<String>,
    pub provider: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetMcpServerRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSkillRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallSkillRequest {
    pub workspace_id: Option<String>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSkillRequest {
    pub workspace_id: Option<String>,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallSkillRequest {
    pub workspace_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSkillsRequest {
    pub workspace_id: Option<String>,
    pub provider: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentGrantKind {
    Mcp,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantAgentCapabilityRequest {
    pub workspace_id: Option<String>,
    pub agent_ref: String,
    pub kind: AgentGrantKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeAgentCapabilityRequest {
    pub agent_ref: String,
    pub kind: AgentGrantKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMcpServersRequest {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListSkillsRequest {
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderAuthStatusRequest {
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartProviderLoginRequest {
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutProviderRequest {
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListProviderProcessesRequest {
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownProviderProcessesRequest {
    pub provider: Option<String>,
    #[serde(default)]
    pub force: bool,
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
pub struct AliasAgentRequest {
    pub session_id: String,
    pub agent_id: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSessionHistoryRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub round_count: Option<usize>,
    pub max_chars: Option<usize>,
    pub before_entry_index: Option<usize>,
    pub before_entry_char_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptInputHistoryEntryKind {
    Prompt,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptInputHistoryEntry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub session_id: String,
    pub source_attachment_id: Option<String>,
    pub kind: PromptInputHistoryEntryKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetPromptInputHistoryRequest {
    pub session_id: String,
    pub after_sequence: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordPromptInputHistoryRequest {
    pub session_id: String,
    pub attachment_id: Option<String>,
    pub kind: PromptInputHistoryEntryKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QueryHistoryRequest {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workflow_id: Option<String>,
    pub machine_id: Option<String>,
    pub repo_root: Option<String>,
    pub worktree_path: Option<String>,
    pub kind: Option<String>,
    pub text: Option<String>,
    pub after_sequence: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHistoryRequest {
    pub query: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workflow_id: Option<String>,
    pub machine_id: Option<String>,
    pub repo_root: Option<String>,
    pub worktree_path: Option<String>,
    pub kind: Option<String>,
    pub after_sequence: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSearchHistoryMode {
    Knn,
    Agent,
}

impl Default for SemanticSearchHistoryMode {
    fn default() -> Self {
        Self::Knn
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSearchHistoryRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<SemanticSearchHistoryMode>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workflow_id: Option<String>,
    pub machine_id: Option<String>,
    pub repo_root: Option<String>,
    pub worktree_path: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticHistoryMatch {
    pub event: HistoryEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_millis: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRuntimeNoticesRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RespondToInteractionRequest {
    pub session_id: String,
    pub interaction_id: String,
    pub choice_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeTerminalRequest {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PumpTerminalOutputRequest {
    pub session_id: String,
    pub attachment_id: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteKernelRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunShellCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDirectoryTreeCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: Option<PathBuf>,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectGitCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub working_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureScreenshotCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreTransferredFileCapabilityRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub source_path: PathBuf,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnAgentRequest {
    pub session_id: String,
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<crate::provider::AgentExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_level: Option<crate::provider::AgentPermissionLevel>,
    pub worktree_id: Option<String>,
    #[serde(default)]
    pub kernel_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_placement: Option<crate::agent::GitWorktreePlacement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveAgentToRemoteRequest {
    pub session_id: String,
    pub agent_ref: String,
    pub machine_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestroyAgentRequest {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusAgentRequest {
    pub session_id: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleAgentFocusRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListAgentsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowRequest {
    pub session_id: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasWorkflowRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveWorkflowRequest {
    pub session_id: String,
    pub workflow_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowPublicationRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowPublicationsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkflowPublicationRequest {
    pub session_id: String,
    pub publication_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisableWorkflowPublicationRequest {
    pub session_id: String,
    pub publication_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowPublicationPairCodeRequest {
    pub session_id: String,
    pub publication_ref: String,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default)]
    pub max_uses: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedeemWorkflowPublicationPairCodeRequest {
    pub session_id: String,
    pub publication_ref: String,
    pub pair_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_transports: Vec<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowPublicationSendersRequest {
    pub session_id: String,
    pub publication_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeWorkflowPublicationSenderRequest {
    pub session_id: String,
    pub publication_ref: String,
    pub sender_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticateWorkflowPublicationSenderRequest {
    pub session_id: String,
    pub publication_ref: String,
    pub credential: String,
    pub transport: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub entry_node_id: String,
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub entry_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddWorkflowNodeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowNodeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWorkflowNodeInstructionsRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeCanCompleteRunRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    pub can_complete_workflow_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeCanEmitIntermediateOutputRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    pub can_emit_intermediate_workflow_run_output: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeIntermediateOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeMaxTurnsRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddWorkflowEdgeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<crate::session::WorkflowOutputValidationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateWorkflowOutputRequest {
    pub session_id: String,
    pub output_schema_ref: String,
    pub output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<crate::session::WorkflowOutputValidationPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWorkflowTurnRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
    pub workflow_node_run_id: String,
    pub delivery_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowEdgeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub edge_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWorkflowCanvasLayoutRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_layout_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patches: Vec<WorkflowCanvasLayoutPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowRunsRequest {
    pub session_id: String,
    pub workflow_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkflowRunRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelWorkflowRunRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeWorkflowRunRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowWatchdogRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub interval_seconds: u64,
    pub invocation_prompt: String,
    pub policy: WorkflowWatchdogPolicy,
    pub max_wakeups_configured: bool,
    pub max_wakeups: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowWatchdogsRequest {
    pub session_id: String,
    pub workflow_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowWatchdogEnabledRequest {
    pub session_id: String,
    pub watchdog_ref: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowWatchdogRequest {
    pub session_id: String,
    pub watchdog_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowFlushContextRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub flush_agent_context_before_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowRunOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowIntermediateOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowLaunchPolicyRequest {
    pub session_id: String,
    pub policy: WorkflowLaunchPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListQueuedWorkflowLaunchesRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveQueuedWorkflowLaunchRequest {
    pub session_id: String,
    pub queue_item_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearQueuedWorkflowLaunchesRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyWorkflowDesignOpRequest {
    pub session_id: String,
    pub origin_client_id: String,
    pub op_id: String,
    pub op: WorkflowDesignOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignWorkflow {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_agent_context_before_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignWorkflowPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_agent_context_before_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignNode {
    pub id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_complete_workflow_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_emit_intermediate_run_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignNodePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_complete_workflow_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_emit_intermediate_run_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<Option<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEdge {
    pub id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<crate::session::WorkflowOutputValidationPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEdgePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema_ref: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<Option<crate::session::WorkflowOutputValidationPolicy>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEndpoint {
    pub id: String,
    pub entry_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEndpointPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowDesignOp {
    WorkflowCreate {
        workflow: WorkflowDesignWorkflow,
    },
    WorkflowUpdate {
        workflow_id: String,
        patch: WorkflowDesignWorkflowPatch,
    },
    WorkflowRemove {
        workflow_id: String,
    },
    NodeAdd {
        workflow_id: String,
        node: WorkflowDesignNode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<WorkflowDesignPoint>,
    },
    NodeUpdate {
        workflow_id: String,
        node_id: String,
        patch: WorkflowDesignNodePatch,
    },
    NodeMove {
        workflow_id: String,
        node_id: String,
        position: WorkflowDesignPoint,
    },
    NodeRemove {
        workflow_id: String,
        node_id: String,
    },
    EdgeAdd {
        workflow_id: String,
        edge: WorkflowDesignEdge,
    },
    EdgeUpdate {
        workflow_id: String,
        edge_id: String,
        patch: WorkflowDesignEdgePatch,
    },
    EdgeRemove {
        workflow_id: String,
        edge_id: String,
    },
    EndpointAdd {
        workflow_id: String,
        endpoint: WorkflowDesignEndpoint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<WorkflowDesignPoint>,
    },
    EndpointUpdate {
        workflow_id: String,
        endpoint_id: String,
        patch: WorkflowDesignEndpointPatch,
    },
    EndpointMove {
        workflow_id: String,
        endpoint_id: String,
        position: WorkflowDesignPoint,
    },
    EndpointRemove {
        workflow_id: String,
        endpoint_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignOpForwarded {
    pub session_id: String,
    pub kernel_sequence: u64,
    pub origin_client_id: String,
    pub op_id: String,
    pub op: WorkflowDesignOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalDaemonRequest {
    CreateSession(CreateSessionRequest),
    AttachToSession(AttachToSessionRequest),
    DetachFromSession(DetachFromSessionRequest),
    ListSessionMembers(ListSessionMembersRequest),
    CreateSessionInvite(CreateSessionInviteRequest),
    JoinSessionInvite(JoinSessionInviteRequest),
    RevokeSessionInvite(RevokeSessionInviteRequest),
    CreateWorkspaceLink(CreateWorkspaceLinkRequest),
    ListWorkspaceLinks(ListWorkspaceLinksRequest),
    ShowWorkspaceLink(ShowWorkspaceLinkRequest),
    AttachWorkspaceLink(AttachWorkspaceLinkRequest),
    DetachWorkspaceLink(DetachWorkspaceLinkRequest),
    LaunchProviderRun(LaunchProviderRunRequest),
    ListSessions(ListSessionsRequest),
    ResolveSession(ResolveSessionRequest),
    GetSessionState(GetSessionStateRequest),
    GetDaemonHealth(GetDaemonHealthRequest),
    GetProviderRun(GetProviderRunRequest),
    UpdateProviderRunSelection(UpdateProviderRunSelectionRequest),
    GetProviderCatalog(GetProviderCatalogRequest),
    GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest),
    InstallMcpServer(InstallMcpServerRequest),
    UpdateMcpServer(UpdateMcpServerRequest),
    UninstallMcpServer(UninstallMcpServerRequest),
    ImportMcpServers(ImportMcpServersRequest),
    GetMcpServer(GetMcpServerRequest),
    ListMcpServers(ListMcpServersRequest),
    InstallSkill(InstallSkillRequest),
    UpdateSkill(UpdateSkillRequest),
    UninstallSkill(UninstallSkillRequest),
    ImportSkills(ImportSkillsRequest),
    GetSkill(GetSkillRequest),
    ListSkills(ListSkillsRequest),
    RelayStatus(RelayStatusRequest),
    ConfigureRelay(ConfigureRelayRequest),
    CloudRelayStatus(CloudRelayStatusRequest),
    StartCloudRelayLogin(StartCloudRelayLoginRequest),
    PollCloudRelayLogin(PollCloudRelayLoginRequest),
    LogoutCloudRelay(LogoutCloudRelayRequest),
    PairCloudRelayClient(PairCloudRelayClientRequest),
    PairCloudRelayMachine(PairCloudRelayMachineRequest),
    ConnectCloudRelay(ConnectCloudRelayRequest),
    IssueCloudRelayClientToken(IssueCloudRelayClientTokenRequest),
    CreateCloudSessionInvite(CreateCloudSessionInviteRequest),
    ShowCloudSessionInvite(ShowCloudSessionInviteRequest),
    AcceptCloudSessionInvite(AcceptCloudSessionInviteRequest),
    RevokeCloudSessionInvite(RevokeCloudSessionInviteRequest),
    ListCloudSessionMembers(ListCloudSessionMembersRequest),
    ListCloudCollaborators(ListCloudCollaboratorsRequest),
    GetUserConfig(GetUserConfigRequest),
    GetUserConfigSchema(GetUserConfigSchemaRequest),
    SetUserConfigValue(SetUserConfigValueRequest),
    UnsetUserConfigValue(UnsetUserConfigValueRequest),
    SetCredentialSecret(SetCredentialSecretRequest),
    DeleteCredentialSecret(DeleteCredentialSecretRequest),
    ListRemoteMachines(ListRemoteMachinesRequest),
    ListRemoteMachineKernels(ListRemoteMachineKernelsRequest),
    GetWaitingRoomInventory(GetWaitingRoomInventoryRequest),
    GetWaitingRoomPublicSnapshot(GetWaitingRoomPublicSnapshotRequest),
    SearchWorkspaceDirectories(SearchWorkspaceDirectoriesRequest),
    CreateWorkspaceDirectory(CreateWorkspaceDirectoryRequest),
    ListWorkspaceWorktrees(ListWorkspaceWorktreesRequest),
    CreateWorkspaceWorktree(CreateWorkspaceWorktreeRequest),
    DeleteWorkspaceWorktree(DeleteWorkspaceWorktreeRequest),
    CreateWorkspacePullRequest(CreateWorkspacePullRequestRequest),
    GetWorkspaceGitOverview(GetWorkspaceGitOverviewRequest),
    ListWorkspaceFiles(ListWorkspaceFilesRequest),
    GetWorkspaceFileContent(GetWorkspaceFileContentRequest),
    RunAgentUtility(RunAgentUtilityRequest),
    GenerateWorkspaceCommitMessage(GenerateWorkspaceCommitMessageRequest),
    CommitWorkspaceChanges(CommitWorkspaceChangesRequest),
    PushWorkspaceBranch(PushWorkspaceBranchRequest),
    CommitAndPushWorkspaceChanges(CommitAndPushWorkspaceChangesRequest),
    ApproveRemoteMachine(ApproveRemoteMachineRequest),
    ForgetRemoteMachine(ForgetRemoteMachineRequest),
    RenameRemoteMachine(RenameRemoteMachineRequest),
    CreatePairingInvite(CreatePairingInviteRequest),
    JoinPairingInvite(JoinPairingInviteRequest),
    CreateTerminalPairingLink(CreateTerminalPairingLinkRequest),
    JoinTerminalPairingLink(JoinTerminalPairingLinkRequest),
    ListTerminals(ListTerminalsRequest),
    ListPairedClients(ListPairedClientsRequest),
    RecordPairedClient(RecordPairedClientRequest),
    RevokePairedClient(RevokePairedClientRequest),
    GetProviderAuthStatus(GetProviderAuthStatusRequest),
    StartProviderLogin(StartProviderLoginRequest),
    LogoutProvider(LogoutProviderRequest),
    ListProviderProcesses(ListProviderProcessesRequest),
    TeardownProviderProcesses(TeardownProviderProcessesRequest),
    GetSessionHistory(GetSessionHistoryRequest),
    GetPromptInputHistory(GetPromptInputHistoryRequest),
    RecordPromptInputHistory(RecordPromptInputHistoryRequest),
    QueryHistory(QueryHistoryRequest),
    SearchHistory(SearchHistoryRequest),
    SemanticSearchHistory(SemanticSearchHistoryRequest),
    PollRuntimeNotices(PollRuntimeNoticesRequest),
    RespondToInteraction(RespondToInteractionRequest),
    SubmitPrompt(SubmitPromptRequest),
    CompletePrompt(CompletePromptRequest),
    CancelActivePrompt(CancelActivePromptRequest),
    UpdateSessionConfig(UpdateSessionConfigRequest),
    UpdateAgentConfig(UpdateAgentConfigRequest),
    UpdateAgentProfile(UpdateAgentProfileRequest),
    UpdateAgentSubstitutes(UpdateAgentSubstitutesRequest),
    ResizeTerminal(ResizeTerminalRequest),
    PumpTerminalOutput(PumpTerminalOutputRequest),
    RunShellCommand(RunShellCapabilityRequest),
    ReadDirectoryTree(ReadDirectoryTreeCapabilityRequest),
    ReadFile(ReadFileCapabilityRequest),
    EditFile(EditFileCapabilityRequest),
    InspectGit(InspectGitCapabilityRequest),
    CaptureScreenshot(CaptureScreenshotCapabilityRequest),
    StoreTransferredFile(StoreTransferredFileCapabilityRequest),
    EndSession(EndSessionRequest),
    DeleteSession(DeleteSessionRequest),
    DeleteKernel(DeleteKernelRequest),
    AliasSession(AliasSessionRequest),
    AliasAgent(AliasAgentRequest),
    SpawnAgent(SpawnAgentRequest),
    MoveAgentToRemote(MoveAgentToRemoteRequest),
    DestroyAgent(DestroyAgentRequest),
    FocusAgent(FocusAgentRequest),
    CycleAgentFocus(CycleAgentFocusRequest),
    GrantAgentCapability(GrantAgentCapabilityRequest),
    RevokeAgentCapability(RevokeAgentCapabilityRequest),
    ListAgents(ListAgentsRequest),
    CreateWorkflow(CreateWorkflowRequest),
    ApplyWorkflowDesignOp(ApplyWorkflowDesignOpRequest),
    AliasWorkflow(AliasWorkflowRequest),
    ListWorkflows(ListWorkflowsRequest),
    ResolveWorkflow(ResolveWorkflowRequest),
    CreateWorkflowPublication(CreateWorkflowPublicationRequest),
    ListWorkflowPublications(ListWorkflowPublicationsRequest),
    GetWorkflowPublication(GetWorkflowPublicationRequest),
    DisableWorkflowPublication(DisableWorkflowPublicationRequest),
    CreateWorkflowPublicationPairCode(CreateWorkflowPublicationPairCodeRequest),
    RedeemWorkflowPublicationPairCode(RedeemWorkflowPublicationPairCodeRequest),
    ListWorkflowPublicationSenders(ListWorkflowPublicationSendersRequest),
    RevokeWorkflowPublicationSender(RevokeWorkflowPublicationSenderRequest),
    AuthenticateWorkflowPublicationSender(AuthenticateWorkflowPublicationSenderRequest),
    CreateWorkflowEndpoint(CreateWorkflowEndpointRequest),
    AliasWorkflowEndpoint(AliasWorkflowEndpointRequest),
    BindWorkflowEndpoint(BindWorkflowEndpointRequest),
    AddWorkflowNode(AddWorkflowNodeRequest),
    RemoveWorkflowNode(RemoveWorkflowNodeRequest),
    UpdateWorkflowNodeInstructions(UpdateWorkflowNodeInstructionsRequest),
    SetWorkflowNodeCanCompleteRun(SetWorkflowNodeCanCompleteRunRequest),
    SetWorkflowNodeCanEmitIntermediateOutput(SetWorkflowNodeCanEmitIntermediateOutputRequest),
    SetWorkflowNodeIntermediateOutputSchema(SetWorkflowNodeIntermediateOutputSchemaRequest),
    SetWorkflowNodeMaxTurns(SetWorkflowNodeMaxTurnsRequest),
    AddWorkflowEdge(AddWorkflowEdgeRequest),
    RemoveWorkflowEdge(RemoveWorkflowEdgeRequest),
    UpdateWorkflowCanvasLayout(UpdateWorkflowCanvasLayoutRequest),
    InvokeWorkflowEndpoint(InvokeWorkflowEndpointRequest),
    ListWorkflowRuns(ListWorkflowRunsRequest),
    GetWorkflowRun(GetWorkflowRunRequest),
    CancelWorkflowRun(CancelWorkflowRunRequest),
    ResumeWorkflowRun(ResumeWorkflowRunRequest),
    CreateWorkflowWatchdog(CreateWorkflowWatchdogRequest),
    ListWorkflowWatchdogs(ListWorkflowWatchdogsRequest),
    SetWorkflowWatchdogEnabled(SetWorkflowWatchdogEnabledRequest),
    RemoveWorkflowWatchdog(RemoveWorkflowWatchdogRequest),
    SetWorkflowFlushContext(SetWorkflowFlushContextRequest),
    SetWorkflowRunOutputSchema(SetWorkflowRunOutputSchemaRequest),
    SetWorkflowIntermediateOutputSchema(SetWorkflowIntermediateOutputSchemaRequest),
    SetWorkflowLaunchPolicy(SetWorkflowLaunchPolicyRequest),
    ListQueuedWorkflowLaunches(ListQueuedWorkflowLaunchesRequest),
    RemoveQueuedWorkflowLaunch(RemoveQueuedWorkflowLaunchRequest),
    ClearQueuedWorkflowLaunches(ClearQueuedWorkflowLaunchesRequest),
    ValidateWorkflowOutput(ValidateWorkflowOutputRequest),
    AckWorkflowTurn(AckWorkflowTurnRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalDaemonResponse {
    SessionCreated {
        session: RuntimeSession,
        agent: AgentInstance,
    },
    SessionAttached {
        attachment: RuntimeAttachment,
    },
    SessionDetached {
        attachment: RuntimeAttachment,
    },
    SessionMembersListed {
        members: Vec<SessionMember>,
        invites: Vec<SessionInvite>,
    },
    SessionInviteCreated {
        invite: SessionInviteRecord,
        session: RuntimeSession,
    },
    SessionInviteJoined {
        member: SessionMember,
        session: RuntimeSession,
    },
    SessionInviteRevoked {
        invite: SessionInvite,
        session: RuntimeSession,
    },
    WorkspaceLinkCreated {
        link: WorkspaceLinkDefinition,
        session: RuntimeSession,
    },
    WorkspaceLinksListed {
        links: Vec<WorkspaceLinkDefinition>,
    },
    WorkspaceLinkShown {
        link: WorkspaceLinkDefinition,
    },
    WorkspaceLinkAttached {
        link: WorkspaceLinkDefinition,
        attachment: WorkspaceLinkAttachment,
        session: RuntimeSession,
    },
    WorkspaceLinkDetached {
        link: WorkspaceLinkDefinition,
        detached: Vec<WorkspaceLinkAttachment>,
        session: RuntimeSession,
    },
    ProviderRunLaunched {
        provider_run: RuntimeProviderRun,
    },
    ProviderRunLaunchAccepted {
        provider_run: RuntimeProviderRun,
    },
    SessionsListed {
        sessions: Vec<RuntimeSession>,
    },
    SessionResolved {
        session: RuntimeSession,
    },
    SessionState {
        session: RuntimeSession,
        agent_activity: BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity>,
    },
    DaemonHealth {
        projection: DaemonHealthProjection,
    },
    ProviderRun {
        provider_run: RuntimeProviderRun,
    },
    ProviderRunSelectionUpdated {
        provider_run: RuntimeProviderRun,
    },
    ProviderCatalog {
        catalog: OpenCodeProviderCatalog,
    },
    ProviderCommandCatalogs {
        catalogs: BTreeMap<String, ProviderCommandCatalog>,
    },
    McpServerInstalled {
        mcp: ArrobaMcpServerConfig,
        path: PathBuf,
    },
    McpServerUpdated {
        mcp: ArrobaMcpServerConfig,
        path: PathBuf,
    },
    McpServerUninstalled {
        name: String,
        path: PathBuf,
    },
    McpServersImported {
        outcome: McpImportOutcome,
    },
    McpServer {
        mcp: ArrobaMcpServerConfig,
    },
    McpServersListed {
        mcps: Vec<ArrobaMcpServerConfig>,
    },
    Skill {
        skill: ArrobaSkillMetadata,
    },
    SkillInstalled {
        skill: ArrobaSkillMetadata,
        path: PathBuf,
    },
    SkillUpdated {
        skill: ArrobaSkillMetadata,
        path: PathBuf,
    },
    SkillUninstalled {
        skill: ArrobaSkillMetadata,
        path: PathBuf,
    },
    SkillsImported {
        outcome: SkillImportOutcome,
    },
    SkillsListed {
        skills: Vec<ArrobaSkillMetadata>,
    },
    RelayStatus {
        status: RelayStatus,
    },
    RelayConfigured {
        status: RelayStatus,
    },
    CloudRelayStatus {
        profile: Option<CloudRelayProfile>,
    },
    CloudRelayLoginStarted {
        login: CloudRelayLoginStart,
    },
    CloudRelayLoginPolled {
        result: CloudRelayLoginPoll,
    },
    CloudRelayLoggedOut,
    CloudRelayClientPaired {
        profile: CloudRelayProfile,
    },
    CloudRelayMachinePaired {
        profile: CloudRelayProfile,
    },
    CloudRelayConnected {
        status: RelayStatus,
        profile: CloudRelayProfile,
        token: CloudRelayRuntimeToken,
    },
    CloudRelayClientTokenIssued {
        profile: CloudRelayProfile,
        token: CloudRelayRuntimeToken,
    },
    CloudSessionInviteCreated {
        invite: CloudSessionInvite,
    },
    CloudSessionInviteShown {
        invite: CloudSessionInviteDetails,
    },
    CloudSessionInviteAccepted {
        acceptance: CloudSessionInviteAcceptance,
    },
    CloudSessionInviteRevoked {
        invite_id: String,
        status: String,
    },
    CloudSessionMembersListed {
        session_id: String,
        members: Vec<CloudSessionMember>,
    },
    CloudCollaboratorsListed {
        collaborators: Vec<CloudCollaborator>,
    },
    UserConfig {
        path: PathBuf,
        config: ArrobaUserConfig,
    },
    UserConfigSchema {
        entries: Vec<crate::config::UserConfigSchemaEntry>,
    },
    UserConfigUpdated {
        path: PathBuf,
        config: ArrobaUserConfig,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<UserConfigMutationEffect>,
    },
    CredentialSecretStored {
        key: String,
    },
    CredentialSecretDeleted {
        key: String,
    },
    RemoteMachinesListed {
        machines: Vec<RemoteMachineRecord>,
    },
    RemoteMachineKernelsListed {
        machine_ref: String,
        kernels: Vec<RelayKernelPresence>,
    },
    WaitingRoomInventory {
        snapshot: WaitingRoomInventorySnapshot,
    },
    WaitingRoomPublicSnapshot {
        snapshot: WaitingRoomPublicSnapshot,
    },
    WorkspaceDirectoriesSearched {
        directories: Vec<String>,
    },
    WorkspaceDirectoryCreated {
        directory: String,
    },
    WorkspaceWorktreesListed {
        workspace_id: String,
        worktrees: Vec<WorkspaceWorktreeRecord>,
    },
    WorkspaceWorktreeCreated {
        workspace_id: String,
        worktree: WorkspaceWorktreeRecord,
    },
    WorkspaceWorktreeDeleted {
        workspace_id: String,
        worktree_id: String,
        path: String,
    },
    WorkspacePullRequestCreated {
        pull_request: WorkspacePullRequestRecord,
    },
    WorkspaceGitOverview {
        overview: WorkspaceGitOverview,
    },
    WorkspaceFilesListed {
        listing: WorkspaceRepoFileListing,
    },
    WorkspaceFileContent {
        content: WorkspaceFileContent,
    },
    WorkspaceFileContentNotModified {
        workspace_id: String,
        worktree_id: String,
        path: String,
        fingerprint: String,
        generated_at_ms: u64,
    },
    AgentUtilityCompleted {
        result: AgentUtilityResult,
    },
    WorkspaceCommitMessageGenerated {
        message: String,
    },
    WorkspaceGitActionCompleted {
        result: WorkspaceGitActionResult,
    },
    RemoteMachineApproved {
        machine: RemoteMachineRecord,
    },
    RemoteMachineForgotten {
        machine: RemoteMachineRecord,
    },
    RemoteMachineRenamed {
        machine: RemoteMachineRecord,
    },
    PairingInviteCreated {
        invite: PairingInviteRecord,
    },
    PairingInviteJoined {
        pairing: PairingJoinRecord,
    },
    TerminalPairingLinkCreated {
        pairing: TerminalPairingLinkRecord,
    },
    TerminalPairingLinkJoined {
        terminal: TerminalRecord,
        pairing: PairingJoinRecord,
    },
    TerminalsListed {
        terminals: Vec<TerminalRecord>,
    },
    PairedClientsListed {
        clients: Vec<PairedClientRecord>,
    },
    PairedClientRecorded {
        client: PairedClientRecord,
    },
    PairedClientRevoked {
        client: PairedClientRecord,
    },
    ProviderAuthStatus {
        status: ProviderAuthStatus,
    },
    ProviderLoginStarted {
        login: ProviderLoginStart,
    },
    ProviderLoggedOut {
        provider: String,
    },
    ProviderProcessesListed {
        processes: Vec<ProviderProcessInfo>,
    },
    ProviderProcessesTornDown {
        processes: Vec<ProviderProcessInfo>,
    },
    SessionHistory {
        entries: Vec<SessionHistoryPageEntry>,
        next_cursor: Option<SessionHistoryCursor>,
    },
    PromptInputHistory {
        entries: Vec<PromptInputHistoryEntry>,
    },
    PromptInputHistoryRecorded {
        entry: PromptInputHistoryEntry,
    },
    HistoryEvents {
        events: Vec<HistoryEvent>,
        next_sequence: Option<u64>,
    },
    SemanticHistoryEvents {
        results: Vec<SemanticHistoryMatch>,
        next_cursor: Option<String>,
        unavailable_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer: Option<String>,
    },
    RuntimeNotices {
        notices: Vec<RuntimeNoticeRecord>,
    },
    InteractionResponded {
        interaction_id: String,
        session: RuntimeSession,
    },
    PromptSubmitted {
        outcome: PromptSubmissionOutcome,
        session: RuntimeSession,
        agent_activity: BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity>,
    },
    PromptCompleted {
        completion: PromptCompletion,
    },
    PromptCancelled {
        cancellation: PromptCancellation,
    },
    SessionConfigUpdated {
        config: SessionConfigState,
        session: RuntimeSession,
    },
    AgentConfigUpdated {
        agent: AgentInstance,
        session: RuntimeSession,
    },
    TerminalResized {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    TerminalOutput {
        records: Vec<TerminalOutputRecord>,
    },
    ShellCommandCompleted {
        result: RunShellCommandResult,
    },
    DirectoryTreeRead {
        result: ReadDirectoryTreeResult,
    },
    FileRead {
        result: ReadFileResult,
    },
    FileEdited {
        result: EditFileResult,
    },
    GitInspected {
        result: InspectGitResult,
    },
    ScreenshotCaptured {
        result: CaptureScreenshotResult,
    },
    FileTransferred {
        result: StoredTransferArtifact,
    },
    SessionEnded {
        session: RuntimeSession,
    },
    SessionDeleted {
        session: RuntimeSession,
    },
    KernelDeleted {
        kernel_id: String,
        deleted_sessions: Vec<RuntimeSession>,
    },
    SessionAliased {
        session: RuntimeSession,
    },
    AgentAliased {
        agent: AgentInstance,
        session: RuntimeSession,
    },
    AgentProfileUpdated {
        agent: AgentInstance,
        session: RuntimeSession,
    },
    AgentSpawned {
        agent: AgentInstance,
    },
    AgentMovedToRemote {
        agent: AgentInstance,
    },
    AgentDestroyed {
        agent: AgentInstance,
    },
    AgentFocused {
        agent: AgentInstance,
    },
    AgentFocusCycled {
        agent: Option<AgentInstance>,
    },
    AgentCapabilityGranted {
        agent: AgentInstance,
    },
    AgentCapabilityRevoked {
        agent: AgentInstance,
    },
    AgentsListed {
        agents: Vec<AgentInstance>,
    },
    WorkflowCreated {
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowAliased {
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowsListed {
        workflows: Vec<WorkflowDefinition>,
    },
    WorkflowResolved {
        workflow: WorkflowDefinition,
    },
    WorkflowPublicationCreated {
        publication: WorkflowPublicationDefinition,
        session: RuntimeSession,
    },
    WorkflowPublicationsListed {
        publications: Vec<WorkflowPublicationDefinition>,
    },
    WorkflowPublication {
        publication: WorkflowPublicationDefinition,
    },
    WorkflowPublicationDisabled {
        publication: WorkflowPublicationDefinition,
        session: RuntimeSession,
    },
    WorkflowPublicationPairCodeCreated {
        pair_code: WorkflowPublicationPairingCodeRecord,
        session: RuntimeSession,
    },
    WorkflowPublicationSenderPaired {
        sender_credential: WorkflowPublicationSenderCredential,
        session: RuntimeSession,
    },
    WorkflowPublicationSendersListed {
        senders: Vec<WorkflowPublicationTrustedSender>,
    },
    WorkflowPublicationSenderRevoked {
        sender: WorkflowPublicationTrustedSender,
        session: RuntimeSession,
    },
    WorkflowPublicationSenderAuthenticated {
        sender: WorkflowPublicationTrustedSender,
    },
    WorkflowDesignOpAccepted {
        session: RuntimeSession,
        event: WorkflowDesignOpForwarded,
    },
    WorkflowDesignOpRejected {
        session_id: String,
        origin_client_id: String,
        op_id: String,
        message: String,
    },
    WorkflowEndpointCreated {
        endpoint: WorkflowEndpointDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowEndpointAliased {
        endpoint: WorkflowEndpointDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowEndpointBound {
        endpoint: WorkflowEndpointDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeAdded {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeRemoved {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeInstructionsUpdated {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeCanCompleteRunUpdated {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeCanEmitIntermediateOutputUpdated {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeIntermediateOutputSchemaUpdated {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowNodeMaxTurnsUpdated {
        node: WorkflowNodeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowEdgeAdded {
        edge: WorkflowEdgeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowEdgeRemoved {
        edge: WorkflowEdgeDefinition,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowCanvasLayoutUpdated {
        layout: WorkflowCanvasLayout,
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowRunInvoked {
        workflow_run: WorkflowRun,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
        session: RuntimeSession,
    },
    WorkflowRunQueued {
        queued_launch: QueuedWorkflowLaunch,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
        session: RuntimeSession,
    },
    WorkflowRunsListed {
        workflow_runs: Vec<WorkflowRun>,
    },
    WorkflowRun {
        workflow_run: WorkflowRun,
    },
    WorkflowRunCancelled {
        workflow_run: WorkflowRun,
        session: RuntimeSession,
    },
    WorkflowRunResumed {
        workflow_run: WorkflowRun,
        session: RuntimeSession,
    },
    WorkflowWatchdogCreated {
        watchdog: WorkflowWatchdogDefinition,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
        session: RuntimeSession,
    },
    WorkflowWatchdogsListed {
        watchdogs: Vec<WorkflowWatchdogDefinition>,
    },
    WorkflowWatchdogUpdated {
        watchdog: WorkflowWatchdogDefinition,
        session: RuntimeSession,
    },
    WorkflowWatchdogRemoved {
        watchdog: WorkflowWatchdogDefinition,
        session: RuntimeSession,
    },
    WorkflowFlushContextUpdated {
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowRunOutputSchemaUpdated {
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowIntermediateOutputSchemaUpdated {
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowLaunchPolicyUpdated {
        session: RuntimeSession,
    },
    QueuedWorkflowLaunchesListed {
        queued_launches: Vec<QueuedWorkflowLaunch>,
    },
    QueuedWorkflowLaunchRemoved {
        queued_launch: QueuedWorkflowLaunch,
        session: RuntimeSession,
    },
    QueuedWorkflowLaunchesCleared {
        queued_launches: Vec<QueuedWorkflowLaunch>,
        session: RuntimeSession,
    },
    WorkflowOutputValidated {
        valid: bool,
        warning: Option<String>,
    },
    WorkflowTurnAcknowledged {
        workflow_run: WorkflowRun,
        session: RuntimeSession,
    },
}

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
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRoomPublicItemActivitySummary {
    pub working: bool,
    pub active_prompt_count: usize,
    pub queued_prompt_count: usize,
    pub error: bool,
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

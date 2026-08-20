use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use crate::app::provider_output::{
    pump_active_prompt_outputs, ProviderOutputPump, ProviderOutputPumpRequest,
};
use crate::attachment::ClientCapabilityLevel;
use crate::local::test_support::LocalRouterTestHarness;
use crate::provider::{
    AgentExecutionMode, AgentPermissionLevel, LaunchProviderRequest, ProviderClientInterface,
    ProviderPromptChunk, ProviderPromptSignalBatch, ProviderRunTokenUsage, RuntimeProviderRun,
};
use crate::session::{
    CreateSessionRequest, PromptSubmissionOutcome, SessionProjectSelection, WorkflowHandoffPayload,
    WorkflowHandoffValidationPolicy, WorkflowNodeRunStatus, WorkflowRunStatus,
    WorkflowTurnRuntimeState,
};
use crate::terminal::TerminalOutputKind;
use crate::{DaemonApp, DaemonConfig, DaemonError};
use chariox_relay::protocol::{
    RelayKernelPresence, RelayMachinePresence, RelayProviderAccountSummary,
};
use sha2::{Digest, Sha256};

use super::{
    AckWorkflowTurnRequest, AddWorkflowEdgeRequest, AddWorkflowNodeRequest, AgentSubstituteAction,
    AliasAgentRequest, AliasSessionRequest, AliasWorkflowEndpointRequest, AliasWorkflowRequest,
    AppendNativeProviderOutputBatchItem, AppendNativeProviderOutputBatchRequest,
    AppendNativeProviderOutputRequest, ApplyWorkflowDesignOpRequest, ArchiveProjectRequest,
    AttachToSessionRequest, AttachWorkspaceLinkRequest, CancelActivePromptRequest,
    CancelWorkflowRunRequest, CaptureScreenshotCapabilityRequest, CommitWorkspaceChangesRequest,
    CompletePromptRequest, CreateSessionInviteRequest, CreateTerminalPairingLinkRequest,
    CreateWorkflowEndpointRequest, CreateWorkflowRequest, CreateWorkspaceLinkRequest,
    CreateWorkspacePullRequestRequest, CreateWorkspaceWorktreeRequest, CycleAgentFocusRequest,
    DeleteProjectRequest, DeleteSessionRequest, DeleteWorkspaceWorktreeRequest,
    DetachFromSessionRequest, DetachWorkspaceLinkRequest, EditFileCapabilityRequest,
    EndSessionRequest, ExportDebugBundleRequest, FocusAgentRequest, GetDaemonHealthRequest,
    GetSessionStateRequest, GetWaitingRoomInventoryRequest, GetWaitingRoomPublicSnapshotRequest,
    GetWorkflowRunRequest, GetWorkspaceFileContentRequest, GetWorkspaceGitOverviewRequest,
    GetWorkspaceLiveSyncStatusRequest, InspectGitCapabilityRequest, InvokeWorkflowEndpointRequest,
    JoinSessionInviteRequest, JoinTerminalPairingLinkRequest, KernelClientConnection,
    LaunchProviderRunRequest, LaunchProviderRunsRequest, ListAgentsRequest, ListProjectsRequest,
    ListQueuedWorkflowPromptsRequest, ListRemoteMachineKernelsRequest, ListRemoteMachinesRequest,
    ListSessionMembersRequest, ListSessionsRequest, ListWorkflowRunsRequest, ListWorkflowsRequest,
    ListWorkspaceFilesRequest, ListWorkspaceLinksRequest, LocalDaemonRequest, LocalDaemonResponse,
    MaterializeWorkflowPublicationRequest, MoveAgentToLocalRequest,
    NativeProviderInteractionResolution, PauseWorkflowRunRequest, PollRuntimeNoticesRequest,
    PushWorkspaceBranchRequest, QueryRecallRequest, ReadDirectoryTreeCapabilityRequest,
    ReadFileCapabilityRequest, RemoteMachineTrustStatus, RemoveWorkflowEdgeRequest,
    RemoveWorkflowNodeRequest, RenameProjectRequest, RequestNativeProviderInteractionRequest,
    ResolveKernelClientConnectionRequest, ResolveSessionRequest, ResolveWorkflowRequest,
    RespondToInteractionRequest, RestoreProjectRequest, ResumeWorkflowRunRequest,
    RevokeSessionInviteRequest, RunShellCapabilityRequest, SemanticRecallMatch,
    SemanticSearchRecallRequest, SendTerminalInputRequest, SetUserConfigValueRequest,
    SetWorkflowNodeCanCompleteRunRequest, SetWorkflowNodeWaitForAllInputsRequest,
    SetWorkspaceLiveSyncModeRequest, ShowWorkspaceLinkRequest, SpawnAgentRequest,
    SpawnAgentsRequest, SpawnAgentsRequestItem, StoreTransferredFileCapabilityRequest,
    SubmitPromptRequest, SubmitPromptsRequest, SubmitPromptsRequestItem, TerminalType,
    UpdateAgentConfigRequest, UpdateAgentProfileRequest, UpdateAgentSubstitutesRequest,
    UpdateProjectWorkspacesRequest, UpdateProviderRunSelectionRequest, UpdateSessionConfigRequest,
    UpdateWorkflowCanvasLayoutRequest, UpdateWorkflowNodeInstructionsRequest,
    UpdateWorkflowPromptQueueRequest, WorkflowDesignNode, WorkflowDesignOp, WorkflowDesignPoint,
    WorkflowPublicationSnapshot, WorkflowPublicationSourceSessionSnapshot, WorkspaceFileContent,
    WorkspacePullRequestRecord, WorkspaceRepoFileEntry, WorkspaceRepoFileListing,
    LOCAL_DAEMON_PROTOCOL_VERSION,
};

mod protocol_shapes;
mod provider_prompt_runtime;
mod remote_inventory;
mod session_control;
mod terminal_output;
mod waiting_room_projection;
mod workflow_definition_control;
mod workflow_run_control;
mod workspace_capabilities;

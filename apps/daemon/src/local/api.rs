use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agent::AgentInstance;
use crate::app::{DaemonApp, SessionHistoryCursor, SessionHistoryPageEntry};
use crate::attachment::{AttachRequest, ClientCapabilityLevel, RuntimeAttachment};
use crate::capability::{
    CaptureScreenshotResult, EditFileResult, InspectGitResult, ReadDirectoryTreeResult,
    ReadFileResult, RunShellCommandRequest, RunShellCommandResult, StoredTransferArtifact,
};
use crate::error::DaemonError;
use crate::provider::{
    OpenCodeProviderCatalog, ProviderAuthStatus, ProviderCommandCatalog, ProviderLoginStart,
    ProviderProcessInfo, RuntimeProviderRun,
};
use crate::session::{
    CreateSessionRequest, PromptAttachment, PromptCancellation, PromptCompletion,
    PromptSubmissionOutcome, QueuedWorkflowLaunch, RuntimeSession, SessionConfigState,
    WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowLaunchPolicy,
    WorkflowNodeDefinition, WorkflowRun, WorkflowWatchdogDefinition, WorkflowWatchdogPolicy,
};
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachFromSessionRequest {
    pub attachment_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitPromptRequest {
    pub session_id: String,
    pub attachment_id: String,
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
pub struct GetSessionStateRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderRunRequest {
    pub provider_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCatalogRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProviderCommandCatalogsRequest;

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
pub struct GetSessionHistoryRequest {
    pub session_id: String,
    pub agent_id: Option<String>,
    pub round_count: Option<usize>,
    pub max_chars: Option<usize>,
    pub before_entry_index: Option<usize>,
    pub before_entry_char_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollRuntimeNoticesRequest {
    pub session_id: String,
    pub attachment_id: String,
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
    pub provider: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub worktree_id: Option<String>,
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
pub struct CreateWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub entry_node_id: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub entry_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddWorkflowNodeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub agent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowNodeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWorkflowNodeInstructionsRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeCanCompleteRunRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    pub can_complete_workflow_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeCanEmitIntermediateOutputRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    pub can_emit_intermediate_workflow_run_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeIntermediateOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeMaxTurnsRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowRunOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowIntermediateOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
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
pub enum LocalDaemonRequest {
    CreateSession(CreateSessionRequest),
    AttachToSession(AttachToSessionRequest),
    DetachFromSession(DetachFromSessionRequest),
    LaunchProviderRun(LaunchProviderRunRequest),
    ListSessions(ListSessionsRequest),
    ResolveSession(ResolveSessionRequest),
    GetSessionState(GetSessionStateRequest),
    GetProviderRun(GetProviderRunRequest),
    GetProviderCatalog(GetProviderCatalogRequest),
    GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest),
    GetProviderAuthStatus(GetProviderAuthStatusRequest),
    StartProviderLogin(StartProviderLoginRequest),
    LogoutProvider(LogoutProviderRequest),
    ListProviderProcesses(ListProviderProcessesRequest),
    TeardownProviderProcesses(TeardownProviderProcessesRequest),
    GetSessionHistory(GetSessionHistoryRequest),
    PollRuntimeNotices(PollRuntimeNoticesRequest),
    SubmitPrompt(SubmitPromptRequest),
    CompletePrompt(CompletePromptRequest),
    CancelActivePrompt(CancelActivePromptRequest),
    UpdateSessionConfig(UpdateSessionConfigRequest),
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
    AliasSession(AliasSessionRequest),
    SpawnAgent(SpawnAgentRequest),
    DestroyAgent(DestroyAgentRequest),
    FocusAgent(FocusAgentRequest),
    CycleAgentFocus(CycleAgentFocusRequest),
    ListAgents(ListAgentsRequest),
    CreateWorkflow(CreateWorkflowRequest),
    AliasWorkflow(AliasWorkflowRequest),
    ListWorkflows(ListWorkflowsRequest),
    ResolveWorkflow(ResolveWorkflowRequest),
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
    ProviderRunLaunched {
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
    },
    ProviderRun {
        provider_run: RuntimeProviderRun,
    },
    ProviderCatalog {
        catalog: OpenCodeProviderCatalog,
    },
    ProviderCommandCatalogs {
        catalogs: BTreeMap<String, ProviderCommandCatalog>,
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
    RuntimeNotices {
        notices: Vec<RuntimeNoticeRecord>,
    },
    PromptSubmitted {
        outcome: PromptSubmissionOutcome,
        session: RuntimeSession,
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
    SessionAliased {
        session: RuntimeSession,
    },
    AgentSpawned {
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

impl DaemonApp {
    fn local_api_session_snapshot(&self, session_id: &str) -> Result<RuntimeSession, DaemonError> {
        let mut session = self.sessions().get_session(session_id)?;
        let agents = self.agents().get_session_agents(session_id);
        session.set_agents(agents);
        Ok(session)
    }

    pub fn handle_local_request(
        &mut self,
        request: LocalDaemonRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        match request {
            LocalDaemonRequest::CreateSession(request) => {
                let (mut session, agent) = self.create_session(request)?;
                // Populate agents list
                let agents = self.agents().get_session_agents(session.id());
                session.set_agents(agents);
                crate::logging::info_with_fields(
                    "daemon.session",
                    "session created with default agent",
                    serde_json::json!({
                        "session_id": session.id(),
                        "session_alias": session.alias(),
                        "workspace_id": session.workspace_id(),
                        "worktree_id": session.worktree_id(),
                        "execution_mode": format!("{:?}", session.execution_mode()),
                        "agent_id": agent.id(),
                        "agent_ref": agent.agent_ref(),
                    }),
                );
                Ok(LocalDaemonResponse::SessionCreated { session, agent })
            }
            LocalDaemonRequest::AttachToSession(request) => {
                Ok(LocalDaemonResponse::SessionAttached {
                    attachment: self.attach(AttachRequest::new(
                        request.session_id,
                        request.client_id,
                        request.capability_level,
                    ))?,
                })
            }
            LocalDaemonRequest::DetachFromSession(request) => {
                Ok(LocalDaemonResponse::SessionDetached {
                    attachment: self.detach(&request.attachment_id)?,
                })
            }
            LocalDaemonRequest::LaunchProviderRun(request) => {
                self.handle_launch_provider_run_request(request)
            }
            LocalDaemonRequest::ListSessions(_) => {
                let sessions = self.sessions().list_sessions();
                // Populate agents for each session
                let sessions_with_agents: Vec<_> = sessions
                    .into_iter()
                    .map(|mut session| {
                        let agents = self.agents().get_session_agents(session.id());
                        session.set_agents(agents);
                        session
                    })
                    .collect();
                Ok(LocalDaemonResponse::SessionsListed {
                    sessions: sessions_with_agents,
                })
            }
            LocalDaemonRequest::ResolveSession(request) => {
                let mut session = self
                    .resolve_session_ref(&request.session_ref, request.workspace_id.as_deref())?;
                // Populate agents list
                let agents = self.agents().get_session_agents(session.id());
                session.set_agents(agents);
                Ok(LocalDaemonResponse::SessionResolved { session })
            }
            LocalDaemonRequest::GetSessionState(request) => {
                let mut session = self.sessions().get_session(&request.session_id)?;
                // Populate agents list from agent service
                let agents = self.agents().get_session_agents(&request.session_id);
                session.set_agents(agents);
                Ok(LocalDaemonResponse::SessionState { session })
            }
            LocalDaemonRequest::GetProviderRun(request) => {
                self.handle_get_provider_run_request(request)
            }
            LocalDaemonRequest::GetProviderCatalog(_) => self.handle_get_provider_catalog_request(),
            LocalDaemonRequest::GetProviderCommandCatalogs(_) => {
                self.handle_get_provider_command_catalogs_request()
            }
            LocalDaemonRequest::GetProviderAuthStatus(request) => {
                self.handle_get_provider_auth_status_request(request)
            }
            LocalDaemonRequest::StartProviderLogin(request) => {
                self.handle_start_provider_login_request(request)
            }
            LocalDaemonRequest::LogoutProvider(request) => {
                self.handle_logout_provider_request(request)
            }
            LocalDaemonRequest::ListProviderProcesses(request) => {
                Ok(LocalDaemonResponse::ProviderProcessesListed {
                    processes: self.list_provider_processes(request.provider.as_deref())?,
                })
            }
            LocalDaemonRequest::TeardownProviderProcesses(request) => {
                Ok(LocalDaemonResponse::ProviderProcessesTornDown {
                    processes: self.teardown_provider_processes(request.provider.as_deref())?,
                })
            }
            LocalDaemonRequest::GetSessionHistory(request) => {
                let page = self.session_history_page(
                    &request.session_id,
                    request.agent_id.as_deref(),
                    request.round_count,
                    request.max_chars,
                    request.before_entry_index,
                    request.before_entry_char_offset,
                )?;
                Ok(LocalDaemonResponse::SessionHistory {
                    entries: page.entries,
                    next_cursor: page.next_cursor,
                })
            }
            LocalDaemonRequest::PollRuntimeNotices(request) => {
                let _ =
                    self.ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
                Ok(LocalDaemonResponse::RuntimeNotices {
                    notices: self
                        .terminal_mut()
                        .drain_notice_records(&request.session_id, &request.attachment_id),
                })
            }
            LocalDaemonRequest::SubmitPrompt(request) => {
                let outcome = crate::transport::TransportService::schedule_direct_prompt(
                    self,
                    &request.session_id,
                    &request.attachment_id,
                    &request.prompt,
                    request.attachments,
                )?;
                let mut session = self.sessions().get_session(&request.session_id)?;
                session.set_agents(self.agents().get_session_agents(&request.session_id));
                Ok(LocalDaemonResponse::PromptSubmitted { outcome, session })
            }
            LocalDaemonRequest::CompletePrompt(request) => {
                Ok(LocalDaemonResponse::PromptCompleted {
                    completion: crate::transport::TransportService::complete_active_prompt(
                        self,
                        &request.session_id,
                    )?,
                })
            }
            LocalDaemonRequest::CancelActivePrompt(request) => {
                Ok(LocalDaemonResponse::PromptCancelled {
                    cancellation: crate::transport::TransportService::cancel_active_prompt(
                        self,
                        &request.session_id,
                        &request.attachment_id,
                    )?,
                })
            }
            LocalDaemonRequest::UpdateSessionConfig(request) => {
                let session_id = request.session_id.clone();
                let config = self.update_session_config(
                    &request.session_id,
                    &request.attachment_id,
                    request.values,
                    request.requires_idle,
                )?;
                let mut session = self.sessions().get_session(&session_id)?;
                session.set_agents(self.agents().get_session_agents(&session_id));
                Ok(LocalDaemonResponse::SessionConfigUpdated { config, session })
            }
            LocalDaemonRequest::ResizeTerminal(request) => {
                self.resize_terminal(&request.session_id, request.cols, request.rows)?;
                Ok(LocalDaemonResponse::TerminalResized {
                    session_id: request.session_id,
                    cols: request.cols,
                    rows: request.rows,
                })
            }
            LocalDaemonRequest::PumpTerminalOutput(request) => {
                Ok(LocalDaemonResponse::TerminalOutput {
                    records: self
                        .pump_terminal_output(&request.session_id, &request.attachment_id)?,
                })
            }
            LocalDaemonRequest::RunShellCommand(request) => {
                Ok(LocalDaemonResponse::ShellCommandCompleted {
                    result: self.run_shell_command(
                        RunShellCommandRequest::new(
                            request.session_id,
                            request.attachment_id,
                            request.command,
                            request.args,
                            PathBuf::new(),
                            request.working_directory,
                        )
                        .with_timeout_ms(request.timeout_ms.unwrap_or(5_000)),
                    )?,
                })
            }
            LocalDaemonRequest::ReadDirectoryTree(request) => {
                Ok(LocalDaemonResponse::DirectoryTreeRead {
                    result: self.read_directory_tree(
                        &request.session_id,
                        &request.attachment_id,
                        request.path,
                        request.max_depth,
                    )?,
                })
            }
            LocalDaemonRequest::ReadFile(request) => Ok(LocalDaemonResponse::FileRead {
                result: self.read_file(
                    &request.session_id,
                    &request.attachment_id,
                    request.path,
                )?,
            }),
            LocalDaemonRequest::EditFile(request) => Ok(LocalDaemonResponse::FileEdited {
                result: self.edit_file(
                    &request.session_id,
                    &request.attachment_id,
                    request.path,
                    request.contents,
                )?,
            }),
            LocalDaemonRequest::InspectGit(request) => Ok(LocalDaemonResponse::GitInspected {
                result: self.inspect_git(
                    &request.session_id,
                    &request.attachment_id,
                    request.working_directory,
                )?,
            }),
            LocalDaemonRequest::CaptureScreenshot(request) => {
                Ok(LocalDaemonResponse::ScreenshotCaptured {
                    result: self.capture_screenshot(&request.session_id, &request.attachment_id)?,
                })
            }
            LocalDaemonRequest::StoreTransferredFile(request) => {
                Ok(LocalDaemonResponse::FileTransferred {
                    result: self.store_transferred_file(
                        &request.session_id,
                        &request.attachment_id,
                        request.source_path,
                        request.display_name,
                    )?,
                })
            }
            LocalDaemonRequest::EndSession(request) => Ok(LocalDaemonResponse::SessionEnded {
                session: self.end_session(&request.session_id)?,
            }),
            LocalDaemonRequest::DeleteSession(request) => Ok(LocalDaemonResponse::SessionDeleted {
                session: self
                    .delete_session_ref(&request.session_ref, request.workspace_id.as_deref())?,
            }),
            LocalDaemonRequest::AliasSession(request) => {
                let _session = self
                    .sessions_mut()
                    .assign_session_alias(&request.session_id, request.alias)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::SessionAliased { session })
            }
            LocalDaemonRequest::SpawnAgent(request) => {
                let create_request =
                    crate::agent::CreateAgentRequest::new(&request.session_id, &request.provider);
                let create_request = if let Some(alias) = request.alias {
                    create_request.with_alias(alias)
                } else {
                    create_request
                };
                let create_request = if let Some(model) = request.model {
                    create_request.with_model(model)
                } else {
                    create_request
                };
                let create_request = if let Some(effort) = request.effort {
                    create_request.with_effort(effort)
                } else {
                    create_request
                };
                let create_request = if let Some(worktree_id) = request.worktree_id {
                    create_request.with_worktree(worktree_id)
                } else {
                    create_request
                };
                let agent = self.spawn_agent(create_request)?;
                Ok(LocalDaemonResponse::AgentSpawned { agent })
            }
            LocalDaemonRequest::DestroyAgent(request) => {
                let agent = self.destroy_agent(&request.agent_id)?;
                Ok(LocalDaemonResponse::AgentDestroyed { agent })
            }
            LocalDaemonRequest::FocusAgent(request) => {
                let agent = self.focus_agent(&request.session_id, &request.agent_id)?;
                Ok(LocalDaemonResponse::AgentFocused { agent })
            }
            LocalDaemonRequest::CycleAgentFocus(request) => {
                let agent = self.cycle_agent_focus(&request.session_id)?;
                Ok(LocalDaemonResponse::AgentFocusCycled { agent })
            }
            LocalDaemonRequest::ListAgents(request) => {
                let agents = self.list_session_agents(&request.session_id);
                Ok(LocalDaemonResponse::AgentsListed { agents })
            }
            LocalDaemonRequest::CreateWorkflow(request) => {
                let workflow = self
                    .sessions_mut()
                    .create_workflow(&request.session_id, request.alias)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowCreated { workflow, session })
            }
            LocalDaemonRequest::AliasWorkflow(request) => {
                let workflow = self.sessions_mut().assign_workflow_alias(
                    &request.session_id,
                    &request.workflow_ref,
                    request.alias,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowAliased { workflow, session })
            }
            LocalDaemonRequest::ListWorkflows(request) => {
                Ok(LocalDaemonResponse::WorkflowsListed {
                    workflows: self.sessions().list_workflows(&request.session_id)?,
                })
            }
            LocalDaemonRequest::ResolveWorkflow(request) => {
                Ok(LocalDaemonResponse::WorkflowResolved {
                    workflow: self
                        .sessions()
                        .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?,
                })
            }
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().create_workflow_endpoint(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.entry_node_id,
                    request.alias,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointCreated {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().assign_workflow_endpoint_alias(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.alias,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointAliased {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let endpoint = self.sessions_mut().bind_workflow_endpoint(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    &request.entry_node_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEndpointBound {
                    endpoint,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let agent_exists = self
                    .agents()
                    .get_session_agents(&request.session_id)
                    .into_iter()
                    .any(|agent| agent.id() == request.agent_id);
                if !agent_exists {
                    return Err(DaemonError::AgentNotFound {
                        agent_id: request.agent_id,
                    });
                }
                let node = self.sessions_mut().add_workflow_node(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.agent_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeAdded {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let node = self.sessions_mut().remove_workflow_node(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeRemoved {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let node = self.sessions_mut().update_workflow_node_instructions(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.instructions.clone(),
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeInstructionsUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let node = self.sessions_mut().set_workflow_node_can_complete_run(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.can_complete_workflow_run,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let node = self
                    .sessions_mut()
                    .set_workflow_node_can_emit_intermediate_output(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.node_id,
                        request.can_emit_intermediate_workflow_run_output,
                    )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let node = self
                    .sessions_mut()
                    .set_workflow_node_intermediate_output_schema_ref(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.node_id,
                        request.intermediate_output_schema_ref.clone(),
                    )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let node = self.sessions_mut().set_workflow_node_max_turns(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.node_id,
                    request.max_turns,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated {
                    node,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let edge = self.sessions_mut().add_workflow_edge(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.from_node_id,
                    &request.to_node_id,
                    request.output_schema_ref.clone(),
                    request.validation_policy,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEdgeAdded {
                    edge,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let edge = self.sessions_mut().remove_workflow_edge(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.edge_id,
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowEdgeRemoved {
                    edge,
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                let outcome = self.invoke_workflow_endpoint_with_admission(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.prompt,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                match outcome {
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    } => Ok(LocalDaemonResponse::WorkflowRunInvoked {
                        workflow_run,
                        workflow,
                        endpoint,
                        session,
                    }),
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                        queued_launch,
                        workflow,
                        endpoint,
                    } => Ok(LocalDaemonResponse::WorkflowRunQueued {
                        queued_launch,
                        workflow,
                        endpoint,
                        session,
                    }),
                }
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                Ok(LocalDaemonResponse::WorkflowRunsListed {
                    workflow_runs: self
                        .sessions()
                        .list_workflow_runs(&request.session_id, request.workflow_ref.as_deref())?,
                })
            }
            LocalDaemonRequest::GetWorkflowRun(request) => Ok(LocalDaemonResponse::WorkflowRun {
                workflow_run: self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?,
            }),
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                let workflow_run_id = self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                    .id()
                    .to_string();
                let should_cancel_active_prompt = self
                    .sessions()
                    .get_session(&request.session_id)?
                    .active_prompt()
                    .and_then(|prompt| prompt.workflow_run_id())
                    == Some(workflow_run_id.as_str());
                if should_cancel_active_prompt {
                    let _ = crate::transport::TransportService::cancel_active_prompt_for_runtime(
                        self,
                        &request.session_id,
                    )?;
                }
                let workflow_run = self
                    .sessions_mut()
                    .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
                let _ = self.drain_session_workflow_launch_queue(&request.session_id)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunCancelled {
                    workflow_run,
                    session,
                })
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                let workflow_run = crate::scheduler::runtime::resume_workflow_run(
                    self,
                    &request.session_id,
                    &request.workflow_run_ref,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunResumed {
                    workflow_run,
                    session,
                })
            }
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                let watchdog = self.sessions_mut().create_workflow_watchdog(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                    request.interval_seconds,
                    request.invocation_prompt,
                    request.policy,
                    if request.max_wakeups_configured {
                        Some(request.max_wakeups)
                    } else {
                        None
                    },
                )?;
                let workflow = self
                    .sessions()
                    .resolve_workflow_ref(&request.session_id, &request.workflow_ref)?;
                let endpoint = self.sessions().resolve_workflow_endpoint_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    &request.endpoint_ref,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogCreated {
                    watchdog,
                    workflow,
                    endpoint,
                    session,
                })
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                Ok(LocalDaemonResponse::WorkflowWatchdogsListed {
                    watchdogs: self.sessions().list_workflow_watchdogs(
                        &request.session_id,
                        request.workflow_ref.as_deref(),
                    )?,
                })
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                let watchdog = self.sessions_mut().set_workflow_watchdog_enabled(
                    &request.session_id,
                    &request.watchdog_ref,
                    request.enabled,
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogUpdated { watchdog, session })
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                let watchdog = self
                    .sessions_mut()
                    .remove_workflow_watchdog(&request.session_id, &request.watchdog_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowWatchdogRemoved { watchdog, session })
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                let workflow = self
                    .sessions_mut()
                    .set_workflow_flush_agent_context_before_run(
                        &request.session_id,
                        &request.workflow_ref,
                        request.flush_agent_context_before_run,
                    )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowFlushContextUpdated { workflow, session })
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                let workflow = self.sessions_mut().set_workflow_run_output_schema_ref(
                    &request.session_id,
                    &request.workflow_ref,
                    request.run_output_schema_ref.clone(),
                )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { workflow, session })
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                let workflow = self
                    .sessions_mut()
                    .set_workflow_intermediate_output_schema_ref(
                        &request.session_id,
                        &request.workflow_ref,
                        request.intermediate_output_schema_ref.clone(),
                    )?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated {
                    workflow,
                    session,
                })
            }
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                let session = self
                    .sessions_mut()
                    .set_workflow_launch_policy(&request.session_id, request.policy)?;
                let mut session = session;
                session.set_agents(self.agents().get_session_agents(&request.session_id));
                Ok(LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session })
            }
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchesListed {
                    queued_launches: self
                        .sessions()
                        .list_queued_workflow_launches(&request.session_id)?,
                })
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                let queued_launch = self
                    .sessions_mut()
                    .remove_queued_workflow_launch(&request.session_id, &request.queue_item_ref)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchRemoved {
                    queued_launch,
                    session,
                })
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                let queued_launches = self
                    .sessions_mut()
                    .clear_queued_workflow_launches(&request.session_id)?;
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::QueuedWorkflowLaunchesCleared {
                    queued_launches,
                    session,
                })
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                let result = crate::transport::runtime_tools::dispatch_runtime_tool_call(
                    self,
                    crate::transport::runtime_tools::RuntimeToolCall {
                        tool_name: crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL
                            .to_string(),
                        arguments: serde_json::json!({
                            "output_schema_ref": request.output_schema_ref,
                            "output_json": request.output_json,
                        }),
                        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                            session_id: request.session_id.clone(),
                            workflow_run_ref: String::new(),
                            workflow_node_run_id: String::new(),
                            delivery_token: None,
                            allowed_output_schema_refs: vec![request.output_schema_ref.clone()],
                            workflow_run_output_schema_ref: None,
                            workflow_intermediate_output_schema_ref: None,
                            can_complete_workflow_run: false,
                            can_emit_intermediate_workflow_run_output: false,
                        },
                    },
                )?;
                Ok(LocalDaemonResponse::WorkflowOutputValidated {
                    valid: result.payload["valid"].as_bool().unwrap_or(false),
                    warning: result.payload["warning"]
                        .as_str()
                        .map(str::to_string)
                        .filter(|value| !value.is_empty()),
                })
            }
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                crate::transport::runtime_tools::dispatch_runtime_tool_call(
                    self,
                    crate::transport::runtime_tools::RuntimeToolCall {
                        tool_name: crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL
                            .to_string(),
                        arguments: serde_json::json!({
                            "delivery_token": request.delivery_token,
                        }),
                        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                            session_id: request.session_id.clone(),
                            workflow_run_ref: request.workflow_run_ref.clone(),
                            workflow_node_run_id: request.workflow_node_run_id.clone(),
                            delivery_token: Some(request.delivery_token.clone()),
                            allowed_output_schema_refs: Vec::new(),
                            workflow_run_output_schema_ref: None,
                            workflow_intermediate_output_schema_ref: None,
                            can_complete_workflow_run: false,
                            can_emit_intermediate_workflow_run_output: false,
                        },
                    },
                )?;
                let workflow_run = self
                    .sessions()
                    .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                    .clone();
                let session = self.local_api_session_snapshot(&request.session_id)?;
                Ok(LocalDaemonResponse::WorkflowTurnAcknowledged {
                    workflow_run,
                    session,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::attachment::ClientCapabilityLevel;
    use crate::session::{
        CreateSessionRequest, PromptSubmissionOutcome, WorkflowHandoffPayload,
        WorkflowOutputValidationPolicy, WorkflowTurnRuntimeState,
    };
    use crate::terminal::TerminalOutputKind;
    use crate::{DaemonApp, DaemonConfig, DaemonError};

    use super::{
        AckWorkflowTurnRequest, AddWorkflowEdgeRequest, AddWorkflowNodeRequest,
        AliasSessionRequest, AliasWorkflowEndpointRequest, AliasWorkflowRequest,
        AttachToSessionRequest, CancelActivePromptRequest, CancelWorkflowRunRequest,
        CaptureScreenshotCapabilityRequest, CompletePromptRequest, CreateWorkflowEndpointRequest,
        CreateWorkflowRequest, CycleAgentFocusRequest, DeleteSessionRequest,
        DetachFromSessionRequest, EditFileCapabilityRequest, EndSessionRequest, FocusAgentRequest,
        GetSessionStateRequest, GetWorkflowRunRequest, InspectGitCapabilityRequest,
        InvokeWorkflowEndpointRequest, LaunchProviderRunRequest, ListAgentsRequest,
        ListSessionsRequest, ListWorkflowRunsRequest, ListWorkflowsRequest, LocalDaemonRequest,
        LocalDaemonResponse, PollRuntimeNoticesRequest, ReadDirectoryTreeCapabilityRequest,
        ReadFileCapabilityRequest, RemoveWorkflowEdgeRequest, RemoveWorkflowNodeRequest,
        ResolveSessionRequest, ResolveWorkflowRequest, ResumeWorkflowRunRequest,
        RunShellCapabilityRequest, SpawnAgentRequest, StoreTransferredFileCapabilityRequest,
        SubmitPromptRequest, UpdateSessionConfigRequest, UpdateWorkflowNodeInstructionsRequest,
    };

    #[test]
    fn local_request_api_supports_session_attach_and_end() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };

        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let detached = match app
            .handle_local_request(LocalDaemonRequest::DetachFromSession(
                DetachFromSessionRequest {
                    attachment_id: attachment.id().to_string(),
                },
            ))
            .expect("detach should succeed")
        {
            LocalDaemonResponse::SessionDetached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let ended = match app
            .handle_local_request(LocalDaemonRequest::EndSession(EndSessionRequest {
                session_id: session.id().to_string(),
            }))
            .expect("end session should succeed")
        {
            LocalDaemonResponse::SessionEnded { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(detached.id(), attachment.id());
        assert_eq!(ended.id(), session.id());
        assert!(app.attachments().get_attachment(detached.id()).is_err());
    }

    #[test]
    fn local_request_api_resolves_and_deletes_sessions_by_ref() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
            _ => panic!("unexpected local response"),
        };

        let resolved = match app
            .handle_local_request(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
                session_ref: "mai".to_string(),
                workspace_id: Some("workspace-1".to_string()),
            }))
            .expect("resolve should succeed")
        {
            LocalDaemonResponse::SessionResolved { session } => session,
            _ => panic!("unexpected local response"),
        };

        let deleted = match app
            .handle_local_request(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: session.id()[..8].to_string(),
                workspace_id: Some("workspace-1".to_string()),
            }))
            .expect("delete should succeed")
        {
            LocalDaemonResponse::SessionDeleted { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(resolved.id(), session.id());
        assert_eq!(deleted.id(), session.id());
        assert_eq!(deleted.alias(), Some("main"));
        assert_eq!(deleted.status(), crate::session::SessionStatus::Ended);
        assert!(matches!(
            app.handle_local_request(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
                session_ref: "main".to_string(),
                workspace_id: Some("workspace-1".to_string()),
            })),
            Err(DaemonError::SessionNotFound { .. })
        ));
        let listed = match app
            .handle_local_request(LocalDaemonRequest::ListSessions(ListSessionsRequest))
            .expect("list should succeed")
        {
            LocalDaemonResponse::SessionsListed { sessions } => sessions,
            _ => panic!("unexpected local response"),
        };
        assert!(listed.is_empty());
    }

    #[test]
    fn local_request_api_aliases_sessions() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };

        let aliased = match app
            .handle_local_request(LocalDaemonRequest::AliasSession(AliasSessionRequest {
                session_id: session.id().to_string(),
                alias: "alpha".to_string(),
            }))
            .expect("alias should succeed")
        {
            LocalDaemonResponse::SessionAliased { session } => session,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(aliased.alias(), Some("alpha"));

        let resolved = match app
            .handle_local_request(LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
                session_ref: "alpha".to_string(),
                workspace_id: Some("workspace-1".to_string()),
            }))
            .expect("alias resolve should succeed")
        {
            LocalDaemonResponse::SessionResolved { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(resolved.id(), session.id());
    }

    #[test]
    fn local_request_api_spawns_and_focuses_agents() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
            _ => panic!("unexpected local response"),
        };

        let spawned = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: "opencode".to_string(),
                model: Some("openai/gpt-5.4".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let session_state = match app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("session state should load")
        {
            LocalDaemonResponse::SessionState { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(session_state.agents().len(), 2);
        assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
        assert_eq!(
            session_state
                .agents()
                .iter()
                .map(|agent| agent.id())
                .collect::<Vec<_>>(),
            vec![default_agent.id(), spawned.id()]
        );
        assert_eq!(
            session_state
                .agents()
                .iter()
                .find(|agent| agent.id() == default_agent.id())
                .expect("default agent should still exist")
                .state(),
            crate::agent::AgentState::Idle
        );
        assert_eq!(
            session_state
                .agents()
                .iter()
                .find(|agent| agent.id() == spawned.id())
                .expect("spawned agent should exist")
                .state(),
            crate::agent::AgentState::Focused
        );

        let focused_default = match app
            .handle_local_request(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: default_agent.id().to_string(),
            }))
            .expect("focus should succeed")
        {
            LocalDaemonResponse::AgentFocused { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(focused_default.id(), default_agent.id());

        let cycled = match app
            .handle_local_request(LocalDaemonRequest::CycleAgentFocus(
                CycleAgentFocusRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("cycle should succeed")
        {
            LocalDaemonResponse::AgentFocusCycled { agent } => {
                agent.expect("cycle should return a focused agent")
            }
            _ => panic!("unexpected local response"),
        };

        assert_eq!(cycled.id(), spawned.id());

        let listed = match app
            .handle_local_request(LocalDaemonRequest::ListAgents(ListAgentsRequest {
                session_id: session.id().to_string(),
            }))
            .expect("list should succeed")
        {
            LocalDaemonResponse::AgentsListed { agents } => agents,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed.iter().map(|agent| agent.id()).collect::<Vec<_>>(),
            vec![default_agent.id(), spawned.id()]
        );
        assert_eq!(
            listed
                .iter()
                .find(|agent| agent.id() == spawned.id())
                .expect("spawned agent should be listed")
                .state(),
            crate::agent::AgentState::Focused
        );
    }

    #[test]
    fn local_request_api_manages_workflows_endpoints_and_graph_edits() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };

        let agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("review".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };

        let listed = match app
            .handle_local_request(LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
                session_id: session.id().to_string(),
            }))
            .expect("workflow list should succeed")
        {
            LocalDaemonResponse::WorkflowsListed { workflows } => workflows,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(listed.len(), 1);

        let resolved = match app
            .handle_local_request(LocalDaemonRequest::ResolveWorkflow(
                ResolveWorkflowRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: "review".to_string(),
                },
            ))
            .expect("workflow resolve should succeed")
        {
            LocalDaemonResponse::WorkflowResolved { workflow } => workflow,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(resolved.id(), workflow.id());

        let node_a = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: agent.id().to_string(),
                },
            ))
            .expect("first workflow node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };

        let duplicate_node = app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: agent.id().to_string(),
                },
            ))
            .expect_err("duplicate workflow node should be rejected");
        assert!(matches!(
            duplicate_node,
            DaemonError::WorkflowNodeConflict { .. }
        ));

        match app
            .handle_local_request(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
                UpdateWorkflowNodeInstructionsRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    node_id: node_a.id().to_string(),
                    instructions: Some("You are the reviewer.".to_string()),
                },
            ))
            .expect("workflow node instructions should update")
        {
            LocalDaemonResponse::WorkflowNodeInstructionsUpdated { node, .. } => {
                assert_eq!(node.instructions(), Some("You are the reviewer."));
            }
            _ => panic!("unexpected local response"),
        };

        let spawned = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer-2".to_string()),
                provider: "opencode".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
            }))
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let node_b = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: spawned.id().to_string(),
                },
            ))
            .expect("second workflow node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };

        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node_a.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("workflow endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(endpoint.entry_node_id(), node_a.id());

        let aliased_workflow = match app
            .handle_local_request(LocalDaemonRequest::AliasWorkflow(AliasWorkflowRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow.id().to_string(),
                alias: "qa".to_string(),
            }))
            .expect("workflow alias should succeed")
        {
            LocalDaemonResponse::WorkflowAliased { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(aliased_workflow.alias(), Some("qa"));

        let aliased_endpoint = match app
            .handle_local_request(LocalDaemonRequest::AliasWorkflowEndpoint(
                AliasWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    alias: "start".to_string(),
                },
            ))
            .expect("workflow endpoint alias should succeed")
        {
            LocalDaemonResponse::WorkflowEndpointAliased { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(aliased_endpoint.alias(), Some("start"));

        let edge = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    from_node_id: node_a.id().to_string(),
                    to_node_id: node_b.id().to_string(),
                    output_schema_ref: None,
                    validation_policy: None,
                },
            ))
            .expect("workflow edge should be added")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
            _ => panic!("unexpected local response"),
        };

        match app
            .handle_local_request(LocalDaemonRequest::RemoveWorkflowEdge(
                RemoveWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    edge_id: edge.id().to_string(),
                },
            ))
            .expect("workflow edge should be removed")
        {
            LocalDaemonResponse::WorkflowEdgeRemoved { .. } => {}
            _ => panic!("unexpected local response"),
        }

        match app
            .handle_local_request(LocalDaemonRequest::RemoveWorkflowNode(
                RemoveWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    node_id: node_a.id().to_string(),
                },
            ))
            .expect("workflow node should be removed")
        {
            LocalDaemonResponse::WorkflowNodeRemoved { .. } => {}
            _ => panic!("unexpected local response"),
        }
    }

    #[test]
    fn local_request_api_invokes_lists_gets_and_cancels_workflow_runs() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };

        let agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("review".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };

        let node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: agent.id().to_string(),
                },
            ))
            .expect("workflow node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };

        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("workflow endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };

        match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "dev-stub".to_string(),
                    account_profile: "default".to_string(),
                    model: "default".to_string(),
                    variant: None,
                },
            ))
            .expect("provider run should launch")
        {
            LocalDaemonResponse::ProviderRunLaunched { .. } => {}
            _ => panic!("unexpected local response"),
        }

        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("review this diff".to_string()),
                },
            ))
            .expect("workflow run invocation should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(workflow_run.workflow_id(), workflow.id());
        assert_eq!(workflow_run.endpoint_id(), endpoint.id());
        assert_eq!(format!("{:?}", workflow_run.status()), "Running");

        let listed = match app
            .handle_local_request(LocalDaemonRequest::ListWorkflowRuns(
                ListWorkflowRunsRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: Some(workflow.id().to_string()),
                },
            ))
            .expect("workflow runs should list")
        {
            LocalDaemonResponse::WorkflowRunsListed { workflow_runs } => workflow_runs,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id(), workflow_run.id());

        let resolved = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(resolved.id(), workflow_run.id());
        assert_eq!(format!("{:?}", resolved.status()), "Running");

        match app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("workflow-backed prompt should complete")
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected local response"),
        }

        let completed = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("completed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(format!("{:?}", completed.status()), "Completed");

        let second_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("review this diff again".to_string()),
                },
            ))
            .expect("second workflow run invocation should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };

        let cancelled = match app
            .handle_local_request(LocalDaemonRequest::CancelWorkflowRun(
                CancelWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: second_run.id().to_string(),
                },
            ))
            .expect("workflow run should cancel")
        {
            LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(cancelled.id(), second_run.id());
        assert_eq!(format!("{:?}", cancelled.status()), "Stopped");
    }

    #[test]
    fn local_request_api_routes_and_schedules_downstream_workflow_nodes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };

        let first_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("planner".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("first workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let second_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("second workflow agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("review".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };

        let first_node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: first_agent.id().to_string(),
                },
            ))
            .expect("first workflow node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };

        let second_node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: second_agent.id().to_string(),
                },
            ))
            .expect("second workflow node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };

        match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    from_node_id: first_node.id().to_string(),
                    to_node_id: second_node.id().to_string(),
                    output_schema_ref: None,
                    validation_policy: None,
                },
            ))
            .expect("workflow edge should be added")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
            _ => panic!("unexpected local response"),
        }

        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: first_node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("workflow endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };

        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("route this workflow".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(format!("{:?}", workflow_run.status()), "Running");
        assert_eq!(workflow_run.node_runs().len(), 1);
        let workflow_attachment_id =
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id());
        let provider_run_id = app
            .sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("workflow invoke should activate a provider run")
            .to_string();
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"planner finished draft plan\",\"output\":{\"message\":\"Please review the attached generated plan and provide approval feedback.\"}}\n```\n",
        );
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderTool,
            None,
            Vec::new(),
            b"{\"tool\":\"rg\",\"status\":\"ok\"}\n",
        );
        let workflow_transfer_root =
            DaemonApp::attachment_artifact_root(session.id(), &workflow_attachment_id, "transfers");
        std::fs::create_dir_all(&workflow_transfer_root)
            .expect("workflow transfer root should exist");
        let workflow_artifact_path = workflow_transfer_root.join("generated-plan.md");
        std::fs::write(&workflow_artifact_path, "# generated plan\n")
            .expect("workflow artifact should be written");

        match app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("entry workflow prompt should complete")
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected local response"),
        }

        let routed = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("routed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(format!("{:?}", routed.status()), "Running");
        assert_eq!(routed.node_runs().len(), 2);
        assert_eq!(routed.messages().len(), 2);
        assert_eq!(
            routed.active_node_run_id(),
            Some(routed.node_runs()[1].id())
        );
        assert_eq!(routed.node_runs()[1].node_id(), second_node.id());
        let completed_entry = routed
            .node_runs()
            .iter()
            .find(|node_run| node_run.node_id() == first_node.id())
            .expect("completed entry node should remain on the run");
        assert_eq!(format!("{:?}", completed_entry.status()), "Completed");
        assert!(completed_entry
            .summary()
            .is_some_and(|summary| summary.contains("planner finished draft plan")));
        let completion = completed_entry
            .completion()
            .expect("completed entry node should retain a generic completion snapshot");
        assert_eq!(completion.summary(), "planner finished draft plan");
        let output = completion
            .output()
            .expect("completed entry node should retain explicit downstream output");
        assert_eq!(
            output.message(),
            "Please review the attached generated plan and provide approval feedback."
        );
        assert_eq!(output.artifacts().len(), 1);
        assert_eq!(output.artifacts()[0].kind(), "transfer");
        assert_eq!(output.artifacts()[0].display_name(), "generated-plan.md");
        assert_eq!(
            output.artifacts()[0].path(),
            workflow_artifact_path.to_string_lossy()
        );
        let handoff_message = routed
            .messages()
            .iter()
            .find(|message| message.source_node_run_id() == Some(completed_entry.id()))
            .expect("downstream handoff message should exist");
        let handoff_payload: WorkflowHandoffPayload =
            serde_json::from_str(handoff_message.handoff_payload())
                .expect("handoff payload should deserialize");
        let handoff_completion = handoff_payload
            .completion()
            .expect("handoff payload should carry the generic completion snapshot");
        assert_eq!(handoff_completion.summary(), "planner finished draft plan");
        let handoff_output = handoff_completion
            .output()
            .expect("handoff payload should carry explicit downstream output");
        assert_eq!(
            handoff_output.message(),
            "Please review the attached generated plan and provide approval feedback."
        );
        assert_eq!(handoff_output.artifacts().len(), 1);
        assert_eq!(
            handoff_output.artifacts()[0].display_name(),
            "generated-plan.md"
        );

        match app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("downstream workflow prompt should complete")
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected local response"),
        }

        let completed = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("completed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(format!("{:?}", completed.status()), "Completed");
        assert_eq!(completed.node_runs().len(), 2);
        assert_eq!(
            completed
                .node_runs()
                .iter()
                .map(|node_run| format!("{:?}", node_run.status()))
                .collect::<Vec<_>>(),
            vec!["Completed".to_string(), "Completed".to_string()]
        );
    }

    #[test]
    fn local_request_api_acks_workflow_turn_and_cleans_up_transient_inputs_after_validation_passes()
    {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-ack", "worktree-ack"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };

        let first_agent = spawn_workflow_test_agent(&mut app, session.id(), "first");
        let second_agent = spawn_workflow_test_agent(&mut app, session.id(), "second");
        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("ack-flow".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let first_node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: first_agent.id().to_string(),
                },
            ))
            .expect("first node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };
        let second_node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: second_agent.id().to_string(),
                },
            ))
            .expect("second node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };
        let _ = app
            .handle_local_request(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
                UpdateWorkflowNodeInstructionsRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    node_id: first_node.id().to_string(),
                    instructions: Some("# First node\nProduce a tiny JSON payload.\n".to_string()),
                },
            ))
            .expect("first node instructions should be updated");
        let _ = app
            .handle_local_request(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
                UpdateWorkflowNodeInstructionsRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    node_id: second_node.id().to_string(),
                    instructions: Some("# Second node\nSummarize the handoff.\n".to_string()),
                },
            ))
            .expect("second node instructions should be updated");
        let _ = app
            .handle_local_request(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    from_node_id: first_node.id().to_string(),
                    to_node_id: second_node.id().to_string(),
                    output_schema_ref: None,
                    validation_policy: None,
                },
            ))
            .expect("edge should be added");
        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: first_node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };

        let (workflow_run, invoke_session) = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("kick off the ack flow".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked {
                workflow_run,
                session,
                ..
            } => (workflow_run, session),
            _ => panic!("unexpected local response"),
        };
        let active_prompt = invoke_session
            .active_prompt()
            .expect("workflow invoke should create an active prompt");
        assert!(active_prompt
            .prompt()
            .contains("Endpoint prompt:\nkick off the ack flow"));
        assert!(active_prompt
            .prompt()
            .contains("Node instruction reference (daemon-managed):"));
        assert!(active_prompt.prompt().contains("`ack_workflow_turn`"));
        assert!(!active_prompt
            .prompt()
            .contains("Control mailbox (daemon-managed):"));

        let first_run_id = workflow_run.node_runs()[0].id().to_string();
        let first_token = "workflow-ack:".to_string() + &first_run_id;
        match app
            .handle_local_request(LocalDaemonRequest::AckWorkflowTurn(
                AckWorkflowTurnRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: first_run_id.clone(),
                    delivery_token: first_token,
                },
            ))
            .expect("workflow turn ack should succeed")
        {
            LocalDaemonResponse::WorkflowTurnAcknowledged { workflow_run, .. } => {
                let envelope = workflow_run.node_runs()[0]
                    .turn_envelope()
                    .expect("first turn envelope should exist");
                assert_eq!(envelope.state(), WorkflowTurnRuntimeState::Acknowledged);
            }
            _ => panic!("unexpected local response"),
        }

        let provider_run_id = app
            .sessions()
            .get_session(session.id())
            .expect("session state should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string();
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"first finished\",\"output\":{\"message\":\"{\\\"value\\\":1}\"}}\n```\n",
        );
        let _ = app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("first workflow prompt should complete");

        let routed = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        let first_completed = routed
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == first_run_id)
            .expect("first node run should remain");
        let first_envelope = first_completed
            .turn_envelope()
            .expect("first node run should retain its envelope");
        assert_eq!(
            first_envelope.state(),
            WorkflowTurnRuntimeState::ValidatedCompleted
        );
        assert!(first_envelope.rendered_prompt().is_none());
        assert!(first_envelope.handoff_payloads_json().is_none());
        assert_eq!(routed.messages().len(), 1);

        let second_active_prompt = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node prompt should be active");
        assert!(second_active_prompt
            .prompt()
            .contains("Workflow handoff payloads (JSON array):"));
        assert!(second_active_prompt
            .prompt()
            .contains("`ack_workflow_turn`"));

        let second_run_id = routed
            .active_node_run_id()
            .expect("second node should be active")
            .to_string();
        let second_token = "workflow-ack:".to_string() + &second_run_id;
        let _ = app
            .handle_local_request(LocalDaemonRequest::AckWorkflowTurn(
                AckWorkflowTurnRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: second_run_id.clone(),
                    delivery_token: second_token,
                },
            ))
            .expect("second workflow turn ack should succeed");
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"second finished\",\"output\":{\"message\":\"{\\\"done\\\":true}\"}}\n```\n",
        );
        let _ = app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("second workflow prompt should complete");
        let completed = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("completed workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(format!("{:?}", completed.status()), "Completed");
        let second_completed = completed
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == second_run_id)
            .expect("second node should complete");
        let second_envelope = second_completed
            .turn_envelope()
            .expect("second node turn envelope should exist");
        assert_eq!(
            second_envelope.state(),
            WorkflowTurnRuntimeState::ValidatedCompleted
        );
        assert!(completed.messages().is_empty());
    }

    #[test]
    fn local_request_api_inlines_mailbox_content_and_retains_inputs_when_validation_warns() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-mailbox", "worktree-mailbox"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };
        let first_agent = spawn_workflow_test_agent(&mut app, session.id(), "loop-a");
        let second_agent = spawn_workflow_test_agent(&mut app, session.id(), "loop-b");
        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("mailbox-flow".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let first_node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: first_agent.id().to_string(),
                },
            ))
            .expect("first node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };
        let second_node = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    agent_id: second_agent.id().to_string(),
                },
            ))
            .expect("second node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        };
        let schema_path = std::env::temp_dir().join(format!(
            "arroba-mailbox-schema-{}.json",
            crate::session::unix_epoch_ms()
        ));
        fs::write(
            &schema_path,
            "{\n  \"type\": \"object\",\n  \"required\": [\"ok\"],\n  \"properties\": {\"ok\": {\"type\": \"boolean\"}}\n}\n",
        )
        .expect("schema file should be written");
        let _ = app
            .handle_local_request(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    from_node_id: first_node.id().to_string(),
                    to_node_id: second_node.id().to_string(),
                    output_schema_ref: Some(schema_path.to_string_lossy().to_string()),
                    validation_policy: Some(WorkflowOutputValidationPolicy::Warn),
                },
            ))
            .expect("first edge should be added");
        let _ = app
            .handle_local_request(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    from_node_id: second_node.id().to_string(),
                    to_node_id: first_node.id().to_string(),
                    output_schema_ref: None,
                    validation_policy: None,
                },
            ))
            .expect("second edge should be added");
        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: first_node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("start loop".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        let node_run_id = workflow_run.node_runs()[0].id().to_string();
        let _ = app
            .handle_local_request(LocalDaemonRequest::AckWorkflowTurn(
                AckWorkflowTurnRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: node_run_id.clone(),
                    delivery_token: format!("workflow-ack:{node_run_id}"),
                },
            ))
            .expect("ack should succeed");
        let provider_run_id = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_provider_run_id()
            .expect("provider run should be active")
            .to_string();
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"warn branch\",\"output\":{\"message\":\"not-json\"}}\n```\n",
        );
        let _ = app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("warning workflow prompt should complete");

        let after_warning = match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session.id().to_string(),
                workflow_run_ref: workflow_run.id().to_string(),
            }))
            .expect("updated workflow run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert!(after_warning.failure_events().iter().any(|event| {
            matches!(
                event.kind(),
                crate::session::WorkflowFailureKind::OutputValidationFailed
            ) && event.message().contains("output.message is not valid JSON")
        }));
        let second_active_prompt = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("second node should be active");
        assert!(second_active_prompt.prompt().contains("Control mailbox:"));
        assert!(second_active_prompt
            .prompt()
            .contains("output.message is not valid JSON"));
        let first_completed = after_warning
            .node_runs()
            .iter()
            .find(|run| run.id() == node_run_id)
            .expect("first node run should remain");
        assert_eq!(
            first_completed
                .turn_envelope()
                .expect("turn envelope should remain")
                .state(),
            WorkflowTurnRuntimeState::Acknowledged
        );
        assert!(first_completed
            .turn_envelope()
            .expect("turn envelope should remain")
            .rendered_prompt()
            .is_some());

        let second_run_id = after_warning
            .active_node_run_id()
            .expect("second node should now be active")
            .to_string();
        let _ = app
            .handle_local_request(LocalDaemonRequest::AckWorkflowTurn(
                AckWorkflowTurnRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: second_run_id.clone(),
                    delivery_token: format!("workflow-ack:{second_run_id}"),
                },
            ))
            .expect("second node ack should succeed");
        app.fan_out_output(
            session.id(),
            &provider_run_id,
            TerminalOutputKind::ProviderOutput,
            None,
            Vec::new(),
            b"```json\n{\"summary\":\"loop back\",\"output\":{\"message\":\"{\\\"ok\\\":true}\"}}\n```\n",
        );
        let _ = app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("second node prompt should complete");

        let active_prompt = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned()
            .expect("first node should be active again");
        assert!(active_prompt.prompt().contains("Control mailbox:"));
        assert!(active_prompt
            .prompt()
            .contains("output.message is not valid JSON"));
        assert!(active_prompt
            .prompt()
            .contains("Treat the control mailbox as authoritative runtime feedback"));
        assert!(active_prompt.prompt().contains("Outgoing edge contracts:"));
        assert!(active_prompt
            .prompt()
            .contains(schema_path.to_string_lossy().as_ref()));
        assert!(!active_prompt
            .prompt()
            .contains("Control mailbox (daemon-managed):"));
    }

    #[test]
    fn local_request_api_resumes_stopped_active_workflow_node_runs() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-resume", "worktree-resume"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };
        let _attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "resume-client".to_string(),
                    capability_level: ClientCapabilityLevel::InteractiveStructured,
                },
            ))
            .expect("attachment should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let agent = spawn_workflow_test_agent(&mut app, session.id(), "resume-node");
        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("resume-flow".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let node = add_workflow_test_node(&mut app, session.id(), workflow.id(), agent.id());
        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("resume prompt".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };

        let cancelled = match app
            .handle_local_request(LocalDaemonRequest::CancelWorkflowRun(
                CancelWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                },
            ))
            .expect("workflow run should stop")
        {
            LocalDaemonResponse::WorkflowRunCancelled { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert_eq!(
            cancelled.status(),
            crate::session::WorkflowRunStatus::Stopped
        );
        assert_eq!(
            app.sessions()
                .get_session(session.id())
                .expect("session should resolve")
                .active_prompt()
                .expect("workflow prompt should be cancelling")
                .status(),
            crate::session::PromptStatus::Cancelling
        );
        let _ = app
            .finalize_active_prompt_cancellation(session.id())
            .expect("workflow cancellation should finalize");
        assert!(app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .is_none());
        let stopped_run = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should resolve after cancellation");
        assert!(stopped_run.failure_events().iter().any(|event| {
            matches!(
                event.kind(),
                crate::session::WorkflowFailureKind::RunStopped
            ) && event
                .message()
                .contains("workflow node run was stopped before validated completion")
        }));

        let resumed = match app
            .handle_local_request(LocalDaemonRequest::ResumeWorkflowRun(
                ResumeWorkflowRunRequest {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                },
            ))
            .expect("workflow run should resume")
        {
            LocalDaemonResponse::WorkflowRunResumed { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };
        assert!(matches!(
            resumed.status(),
            crate::session::WorkflowRunStatus::Waiting
                | crate::session::WorkflowRunStatus::Running
                | crate::session::WorkflowRunStatus::Completed
        ));
        let active_prompt = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve")
            .active_prompt()
            .cloned();
        if let Some(active_prompt) = active_prompt {
            assert!(active_prompt.prompt().contains("resume prompt"));
        }
        let resumed_run = resumed
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_run.node_runs()[0].id())
            .expect("node run should remain");
        assert!(matches!(
            resumed_run.status(),
            crate::session::WorkflowNodeRunStatus::Ready
                | crate::session::WorkflowNodeRunStatus::Running
                | crate::session::WorkflowNodeRunStatus::Completed
        ));
        assert!(resumed_run
            .turn_envelope()
            .and_then(|envelope| envelope.rendered_prompt())
            .is_some());
    }

    #[test]
    fn local_request_api_rejects_workflow_run_when_agent_lacks_required_control_capability() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-control", "worktree-control"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };
        let unsupported_agent = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("unsupported-node".to_string()),
                provider: "dev-invalid-pty".to_string(),
                model: Some("default".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("agent spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };
        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("control-check".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };
        let node = add_workflow_test_node(
            &mut app,
            session.id(),
            workflow.id(),
            unsupported_agent.id(),
        );
        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("endpoint create should succeed")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };
        let error = app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("hello".to_string()),
                },
            ))
            .expect_err("workflow invoke should fail when controls are unsupported");
        assert!(matches!(
            error,
            DaemonError::WorkflowNodeControlUnsupported { operation, .. }
                if operation == "ack_workflow_turn"
        ));
    }

    #[test]
    fn local_request_api_waits_for_all_join_inputs_before_scheduling_downstream_node() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, .. } => session,
            _ => panic!("unexpected local response"),
        };

        let entry_agent = spawn_workflow_test_agent(&mut app, session.id(), "entry");
        let branch_one_agent = spawn_workflow_test_agent(&mut app, session.id(), "branch-one");
        let branch_two_agent = spawn_workflow_test_agent(&mut app, session.id(), "branch-two");
        let join_agent = spawn_workflow_test_agent(&mut app, session.id(), "join");

        let workflow = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("join".to_string()),
            }))
            .expect("workflow create should succeed")
        {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
            _ => panic!("unexpected local response"),
        };

        let entry_node =
            add_workflow_test_node(&mut app, session.id(), workflow.id(), entry_agent.id());
        let branch_one_node =
            add_workflow_test_node(&mut app, session.id(), workflow.id(), branch_one_agent.id());
        let branch_two_node =
            add_workflow_test_node(&mut app, session.id(), workflow.id(), branch_two_agent.id());
        let join_node =
            add_workflow_test_node(&mut app, session.id(), workflow.id(), join_agent.id());
        add_workflow_test_edge(
            &mut app,
            session.id(),
            workflow.id(),
            entry_node.id(),
            branch_one_node.id(),
        );
        add_workflow_test_edge(
            &mut app,
            session.id(),
            workflow.id(),
            entry_node.id(),
            branch_two_node.id(),
        );
        add_workflow_test_edge(
            &mut app,
            session.id(),
            workflow.id(),
            branch_one_node.id(),
            join_node.id(),
        );
        add_workflow_test_edge(
            &mut app,
            session.id(),
            workflow.id(),
            branch_two_node.id(),
            join_node.id(),
        );

        let endpoint = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    entry_node_id: entry_node.id().to_string(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("workflow endpoint should be created")
        {
            LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
            _ => panic!("unexpected local response"),
        };

        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow.id().to_string(),
                    endpoint_ref: endpoint.id().to_string(),
                    prompt: Some("run the join drill".to_string()),
                },
            ))
            .expect("workflow invoke should succeed")
        {
            LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            _ => panic!("unexpected local response"),
        };

        complete_workflow_test_prompt(&mut app, session.id(), "entry workflow prompt");
        let after_entry = get_workflow_test_run(&mut app, session.id(), workflow_run.id());
        assert_eq!(after_entry.node_runs().len(), 3);
        let session_after_entry = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve after entry");
        assert!(session_after_entry.active_prompt().is_some());
        assert_eq!(session_after_entry.queued_prompts().len(), 1);

        complete_workflow_test_prompt(&mut app, session.id(), "first branch workflow prompt");
        let after_first_branch = get_workflow_test_run(&mut app, session.id(), workflow_run.id());
        assert_eq!(after_first_branch.node_runs().len(), 3);
        assert!(after_first_branch
            .node_runs()
            .iter()
            .all(|node_run| node_run.node_id() != join_node.id()));
        let buffered_join_messages = after_first_branch
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join_node.id())
            .collect::<Vec<_>>();
        assert_eq!(buffered_join_messages.len(), 1);
        assert!(buffered_join_messages[0]
            .consumed_by_node_run_id()
            .is_none());
        let session_after_first_branch = app
            .sessions()
            .get_session(session.id())
            .expect("session should resolve after first branch");
        assert!(
            session_after_first_branch.active_prompt().is_some(),
            "expected the second branch prompt to be active after the first branch completed"
        );
        assert_eq!(session_after_first_branch.queued_prompts().len(), 0);

        complete_workflow_test_prompt(&mut app, session.id(), "second branch workflow prompt");
        let after_second_branch = get_workflow_test_run(&mut app, session.id(), workflow_run.id());
        let join_runs = after_second_branch
            .node_runs()
            .iter()
            .filter(|node_run| node_run.node_id() == join_node.id())
            .collect::<Vec<_>>();
        assert_eq!(join_runs.len(), 1);
        let join_run = join_runs[0];
        let join_messages = after_second_branch
            .messages()
            .iter()
            .filter(|message| message.target_node_id() == join_node.id())
            .collect::<Vec<_>>();
        assert_eq!(join_messages.len(), 2);
        assert!(join_messages
            .iter()
            .all(|message| message.consumed_by_node_run_id() == Some(join_run.id())));

        complete_workflow_test_prompt(&mut app, session.id(), "join workflow prompt");
        let completed = get_workflow_test_run(&mut app, session.id(), workflow_run.id());
        assert_eq!(format!("{:?}", completed.status()), "Completed");
        assert_eq!(completed.node_runs().len(), 4);
    }

    #[test]
    fn detaching_one_attachment_keeps_the_session_open_for_others() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };

        let first = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("first attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let second = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-2".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("second attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let detached = match app
            .handle_local_request(LocalDaemonRequest::DetachFromSession(
                DetachFromSessionRequest {
                    attachment_id: first.id().to_string(),
                },
            ))
            .expect("detach should succeed")
        {
            LocalDaemonResponse::SessionDetached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let state = match app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("state request should succeed")
        {
            LocalDaemonResponse::SessionState { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(detached.id(), first.id());
        assert_eq!(state.status().to_string(), "created");
        assert_eq!(state.attachment_ids().len(), 1);
        assert!(state.has_attachment(second.id()));
        assert!(app.attachments().get_attachment(second.id()).is_ok());
    }

    #[test]
    fn focusing_another_agent_during_a_prompt_keeps_the_working_run_active() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, default_agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let default_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(default_agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("default provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let spawned = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("reviewer".to_string()),
                provider: "claude-code".to_string(),
                model: None,
                effort: None,
                worktree_id: None,
            }))
            .expect("spawn should succeed")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        };

        let focused_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(spawned.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "opus".to_string(),
                    variant: None,
                },
            ))
            .expect("spawned provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let _ = app
            .handle_local_request(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: default_agent.id().to_string(),
            }))
            .expect("focusing default agent should succeed");

        let started = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "keep streaming while focus changes\n".to_string(),
                attachments: Vec::new(),
            }))
            .expect("prompt should start");

        match started {
            LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), default_agent.id());
                }
                _ => panic!("expected prompt to start immediately"),
            },
            _ => panic!("unexpected local response"),
        }

        let _ = app
            .handle_local_request(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: session.id().to_string(),
                agent_id: spawned.id().to_string(),
            }))
            .expect("focusing spawned agent should succeed");

        let session_state = match app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("session state should load")
        {
            LocalDaemonResponse::SessionState { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
        assert_eq!(
            session_state.active_provider_run_id(),
            Some(default_run.id())
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_output = false;
        while Instant::now() < deadline {
            let records = app
                .pump_terminal_output(session.id(), attachment.id())
                .expect("terminal output should keep pumping");
            if !records.is_empty() {
                saw_output = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            saw_output,
            "expected background agent output to continue while unfocused"
        );

        let settle_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = app
                .pump_terminal_output(session.id(), attachment.id())
                .expect("terminal output should keep pumping");
            let session_state = app
                .sessions()
                .get_session(session.id())
                .expect("session should still exist");
            if session_state.active_prompt().is_none() {
                assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
                assert_eq!(
                    session_state.active_provider_run_id(),
                    Some(focused_run.id())
                );
                break;
            }
            assert!(
                Instant::now() < settle_deadline,
                "prompt did not settle in time"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn attaching_the_same_client_replaces_its_stale_attachment() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };

        let first = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("first attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let second = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("second attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let state = match app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("state request should succeed")
        {
            LocalDaemonResponse::SessionState { session } => session,
            _ => panic!("unexpected local response"),
        };

        assert_ne!(first.id(), second.id());
        assert_eq!(state.attachment_ids().len(), 1);
        assert!(state.has_attachment(second.id()));
        assert!(app.attachments().get_attachment(first.id()).is_err());
    }

    #[test]
    fn local_request_api_rejects_prompt_without_active_provider_run() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "whoami".to_string(),
                attachments: Vec::new(),
            }))
            .expect_err("prompt submit should fail without active provider run");

        match error {
            DaemonError::NoActiveProviderRun { session_id } => assert_eq!(session_id, session.id()),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_rejects_invalid_provider_adapter() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: None,
                    adapter_key: "missing-adapter".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect_err("unknown adapters should be rejected");

        match error {
            DaemonError::ProviderAdapterNotFound { adapter_key } => {
                assert_eq!(adapter_key, "missing-adapter")
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_exposes_queue_config_and_notices() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let a = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-a".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let b = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-b".to_string(),
                    capability_level: ClientCapabilityLevel::InteractiveStructured,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let _ = app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: None,
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider launch should succeed");

        let first = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
                prompt: "first".to_string(),
                attachments: Vec::new(),
            }))
            .expect("first prompt should start");
        let second = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: b.id().to_string(),
                prompt: "second".to_string(),
                attachments: Vec::new(),
            }))
            .expect("second prompt should queue");
        let config = app
            .handle_local_request(LocalDaemonRequest::UpdateSessionConfig(
                UpdateSessionConfigRequest {
                    session_id: session.id().to_string(),
                    attachment_id: a.id().to_string(),
                    values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                    requires_idle: false,
                },
            ))
            .expect("config update should succeed");

        match first {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                session,
            } => {
                assert!(session.active_prompt().is_some());
            }
            _ => panic!("unexpected first prompt response"),
        }
        match second {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Queued { .. },
                session,
            } => {
                assert_eq!(session.queued_prompts().len(), 1);
            }
            _ => panic!("unexpected second prompt response"),
        }
        match config {
            LocalDaemonResponse::SessionConfigUpdated { config, session } => {
                assert_eq!(config.version(), 1);
                assert_eq!(session.config_state().version(), 1);
            }
            _ => panic!("unexpected config response"),
        }

        let notices = app
            .handle_local_request(LocalDaemonRequest::PollRuntimeNotices(
                PollRuntimeNoticesRequest {
                    session_id: session.id().to_string(),
                    attachment_id: b.id().to_string(),
                },
            ))
            .expect("notice polling should succeed");
        match notices {
            LocalDaemonResponse::RuntimeNotices { notices } => assert!(!notices.is_empty()),
            _ => panic!("unexpected notices response"),
        }

        let state = app
            .handle_local_request(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("state request should succeed");
        match state {
            LocalDaemonResponse::SessionState { session } => {
                assert_eq!(session.queued_prompts().len(), 1);
                assert_eq!(session.config_state().version(), 1);
            }
            _ => panic!("unexpected state response"),
        }

        let completed = app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session.id().to_string(),
            }))
            .expect("complete prompt should succeed");
        match completed {
            LocalDaemonResponse::PromptCompleted { completion } => {
                assert!(completion.started_next.is_some())
            }
            _ => panic!("unexpected completion response"),
        }
    }

    #[test]
    fn local_request_api_can_cancel_an_active_prompt() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");

        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };

        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-a".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let _provider_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: None,
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let _ = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "first prompt\n".to_string(),
                attachments: Vec::new(),
            }))
            .expect("first prompt should start");
        let _ = app
            .handle_local_request(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                prompt: "second prompt\n".to_string(),
                attachments: Vec::new(),
            }))
            .expect("second prompt should queue");

        let response = app
            .handle_local_request(LocalDaemonRequest::CancelActivePrompt(
                CancelActivePromptRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                },
            ))
            .expect("cancel should succeed");

        match response {
            LocalDaemonResponse::PromptCancelled { cancellation } => {
                assert_eq!(
                    cancellation.prompt.status(),
                    crate::session::PromptStatus::Cancelling
                );
                assert!(cancellation.started_next.is_none());
            }
            _ => panic!("unexpected local response"),
        }
    }

    #[test]
    fn local_request_api_runs_shell_command_capability() {
        let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-test");
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-shell".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let response = app
            .handle_local_request(LocalDaemonRequest::RunShellCommand(
                RunShellCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    command: "/bin/sh".to_string(),
                    args: vec!["-lc".to_string(), "printf capability".to_string()],
                    working_directory: None,
                    timeout_ms: None,
                },
            ))
            .expect("shell capability should succeed");

        match response {
            LocalDaemonResponse::ShellCommandCompleted { result } => {
                assert_eq!(result.exit_code, 0);
                assert_eq!(result.stdout, "capability");
            }
            _ => panic!("unexpected shell response"),
        }
    }

    #[test]
    fn local_request_api_rejects_shell_command_for_unauthorized_attachment() {
        let worktree_root = std::env::temp_dir().join("arroba-shell-local-api-denied-test");
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-automation".to_string(),
                    capability_level: ClientCapabilityLevel::AutomationOnly,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::RunShellCommand(
                RunShellCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    command: "/bin/sh".to_string(),
                    args: vec!["-lc".to_string(), "printf denied".to_string()],
                    working_directory: None,
                    timeout_ms: None,
                },
            ))
            .expect_err("automation-only attachment should not run shell commands");

        match error {
            DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
                assert_eq!(session_id, session.id());
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_rejects_file_capability_for_unauthorized_attachment() {
        let worktree_root = std::env::temp_dir().join("arroba-file-local-api-denied-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        std::fs::write(worktree_root.join("notes.txt"), "hello").expect("file should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-automation".to_string(),
                    capability_level: ClientCapabilityLevel::AutomationOnly,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let error = app
            .handle_local_request(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: worktree_root.join("notes.txt"),
            }))
            .expect_err("automation-only attachment should not read files");

        match error {
            DaemonError::AttachmentCapabilityDenied { session_id, .. } => {
                assert_eq!(session_id, session.id());
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn local_request_api_reads_directory_tree_file_and_git_status() {
        let worktree_root = std::env::temp_dir().join("arroba-capability-local-api-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(worktree_root.join("src")).expect("worktree should exist");
        std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
        std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&worktree_root)
            .output()
            .expect("git init should work");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-capability".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let tree = app
            .handle_local_request(LocalDaemonRequest::ReadDirectoryTree(
                ReadDirectoryTreeCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    path: None,
                    max_depth: 2,
                },
            ))
            .expect("tree read should succeed");
        let file = app
            .handle_local_request(LocalDaemonRequest::ReadFile(ReadFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: worktree_root.join("src/lib.rs"),
            }))
            .expect("file read should succeed");
        let edit = app
            .handle_local_request(LocalDaemonRequest::EditFile(EditFileCapabilityRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                path: worktree_root.join("src/lib.rs"),
                contents: "after".to_string(),
            }))
            .expect("file edit should succeed");
        let git = app
            .handle_local_request(LocalDaemonRequest::InspectGit(
                InspectGitCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    working_directory: None,
                },
            ))
            .expect("git inspect should succeed");

        match tree {
            LocalDaemonResponse::DirectoryTreeRead { result } => {
                assert!(result
                    .entries
                    .iter()
                    .any(|entry| entry.relative_path == "README.md"));
            }
            _ => panic!("unexpected tree response"),
        }
        match file {
            LocalDaemonResponse::FileRead { result } => assert_eq!(result.contents, "before"),
            _ => panic!("unexpected file response"),
        }
        match edit {
            LocalDaemonResponse::FileEdited { result } => {
                assert_eq!(result.bytes_written, 5);
                assert_eq!(result.old_size, 6);
                assert_eq!(result.new_size, 5);
                assert!(result.changed);
            }
            _ => panic!("unexpected edit response"),
        }
        match git {
            LocalDaemonResponse::GitInspected { result } => assert!(result.status.contains("main")),
            _ => panic!("unexpected git response"),
        }
    }

    #[test]
    fn local_request_api_returns_structured_screenshot_unavailable_result() {
        std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(
                    "workspace-1",
                    std::env::temp_dir().display().to_string(),
                ),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-screenshot".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let response = app
            .handle_local_request(LocalDaemonRequest::CaptureScreenshot(
                CaptureScreenshotCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                },
            ))
            .expect("screenshot request should succeed with unavailable result");
        std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

        match response {
            LocalDaemonResponse::ScreenshotCaptured { result } => {
                assert_eq!(
                    result.status,
                    crate::capability::ScreenshotStatus::Unavailable
                );
            }
            _ => panic!("unexpected screenshot response"),
        }
    }

    #[test]
    fn local_request_api_stores_transferred_file_under_session_artifacts() {
        let worktree_root = std::env::temp_dir().join("arroba-transfer-local-api-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
        let source = worktree_root.join("artifact.txt");
        std::fs::write(&source, "artifact").expect("file should exist");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", worktree_root.display().to_string()),
            ))
            .expect("session create should succeed")
        {
            LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
            _ => panic!("unexpected local response"),
        };
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "client-transfer".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };

        let response = app
            .handle_local_request(LocalDaemonRequest::StoreTransferredFile(
                StoreTransferredFileCapabilityRequest {
                    session_id: session.id().to_string(),
                    attachment_id: attachment.id().to_string(),
                    source_path: source,
                    display_name: None,
                },
            ))
            .expect("transfer should succeed");

        match response {
            LocalDaemonResponse::FileTransferred { result } => {
                assert!(result
                    .stored_path
                    .to_string_lossy()
                    .contains("arroba-session-artifacts"));
                assert_eq!(result.bytes, 8);
            }
            _ => panic!("unexpected transfer response"),
        }
    }

    fn spawn_workflow_test_agent(
        app: &mut DaemonApp,
        session_id: &str,
        alias: &str,
    ) -> crate::agent::AgentInstance {
        match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session_id.to_string(),
                alias: Some(alias.to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                worktree_id: None,
            }))
            .expect("workflow test agent should spawn")
        {
            LocalDaemonResponse::AgentSpawned { agent } => agent,
            _ => panic!("unexpected local response"),
        }
    }

    fn add_workflow_test_node(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_id: &str,
        agent_id: &str,
    ) -> crate::session::WorkflowNodeDefinition {
        match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session_id.to_string(),
                    workflow_ref: workflow_id.to_string(),
                    agent_id: agent_id.to_string(),
                },
            ))
            .expect("workflow test node should be added")
        {
            LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
            _ => panic!("unexpected local response"),
        }
    }

    fn add_workflow_test_edge(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_id: &str,
        from_node_id: &str,
        to_node_id: &str,
    ) {
        match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowEdge(
                AddWorkflowEdgeRequest {
                    session_id: session_id.to_string(),
                    workflow_ref: workflow_id.to_string(),
                    from_node_id: from_node_id.to_string(),
                    to_node_id: to_node_id.to_string(),
                    output_schema_ref: None,
                    validation_policy: None,
                },
            ))
            .expect("workflow test edge should be added")
        {
            LocalDaemonResponse::WorkflowEdgeAdded { .. } => {}
            _ => panic!("unexpected local response"),
        }
    }

    fn complete_workflow_test_prompt(app: &mut DaemonApp, session_id: &str, label: &str) {
        match app
            .handle_local_request(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
                session_id: session_id.to_string(),
            }))
            .unwrap_or_else(|error| panic!("{label} should complete: {error}"))
        {
            LocalDaemonResponse::PromptCompleted { .. } => {}
            _ => panic!("unexpected local response"),
        }
    }

    fn get_workflow_test_run(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run_id: &str,
    ) -> crate::session::WorkflowRun {
        match app
            .handle_local_request(LocalDaemonRequest::GetWorkflowRun(GetWorkflowRunRequest {
                session_id: session_id.to_string(),
                workflow_run_ref: workflow_run_id.to_string(),
            }))
            .expect("workflow test run should resolve")
        {
            LocalDaemonResponse::WorkflowRun { workflow_run } => workflow_run,
            _ => panic!("unexpected local response"),
        }
    }
}

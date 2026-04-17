use super::*;

use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};
use arroba_relay::protocol::RelayKernelPresence;

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
    #[serde(default)]
    pub machine_ref: Option<String>,
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
    GetDaemonHealth(GetDaemonHealthRequest),
    GetProviderRun(GetProviderRunRequest),
    GetProviderCatalog(GetProviderCatalogRequest),
    GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest),
    InstallMcpServer(InstallMcpServerRequest),
    GetMcpServer(GetMcpServerRequest),
    ListMcpServers(ListMcpServersRequest),
    InstallSkill(InstallSkillRequest),
    GetSkill(GetSkillRequest),
    ListSkills(ListSkillsRequest),
    RelayStatus(RelayStatusRequest),
    ConfigureRelay(ConfigureRelayRequest),
    ListRemoteMachines(ListRemoteMachinesRequest),
    ListRemoteMachineKernels(ListRemoteMachineKernelsRequest),
    ApproveRemoteMachine(ApproveRemoteMachineRequest),
    ForgetRemoteMachine(ForgetRemoteMachineRequest),
    RenameRemoteMachine(RenameRemoteMachineRequest),
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
    },
    DaemonHealth {
        projection: DaemonHealthProjection,
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
    McpServerInstalled {
        mcp: ArrobaMcpServerConfig,
        path: PathBuf,
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
    SkillsListed {
        skills: Vec<ArrobaSkillMetadata>,
    },
    RelayStatus {
        status: RelayStatus,
    },
    RelayConfigured {
        status: RelayStatus,
    },
    RemoteMachinesListed {
        machines: Vec<RemoteMachineRecord>,
    },
    RemoteMachineKernelsListed {
        machine_ref: String,
        kernels: Vec<RelayKernelPresence>,
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

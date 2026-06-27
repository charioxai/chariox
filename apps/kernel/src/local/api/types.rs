use super::*;

use crate::session::{
    PromptQueueItem, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
    RuntimeInteractionCustomChoice, RuntimeInteractionLevel,
};
use crate::slice::{
    SliceBackendKind, SliceDisplayEndpoint, SliceLogEntry, SliceProviderLoginStart, SliceRecord,
};
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputKind, TerminalOutputRecord};
use arroba_relay::protocol::RelayKernelPresence;

mod agent_lifecycle;
mod agent_utility;
mod capability;
mod cloud_relay;
mod config_capabilities;
mod external_provider_session;
mod history;
mod prompt_control;
mod provider_control;
mod remote_access;
mod session_control;
mod slice;
mod terminal_interaction;
mod waiting_room;
mod workflow;
mod workspace;

pub use agent_lifecycle::*;
pub use agent_utility::*;
pub use capability::*;
pub use cloud_relay::*;
pub use config_capabilities::*;
pub use external_provider_session::*;
pub use history::*;
pub use prompt_control::*;
pub use provider_control::*;
pub use remote_access::*;
pub use session_control::*;
pub use slice::*;
pub use terminal_interaction::*;
pub use waiting_room::*;
pub use workflow::*;
pub use workspace::*;

pub const LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 201;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDaemonHealthRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportDebugBundleRequest {
    pub session_id: String,
    pub bundle_label: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWaitingRoomInventoryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWaitingRoomPublicSnapshotRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteKernelRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMetaagentEventsRequest {
    pub session_id: String,
    pub metaagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchMetaagentCommandsRequest {
    pub session_id: String,
    pub metaagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutates: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GetMetaagentTurnOverviewRequest {
    pub session_id: String,
    pub metaagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns_back: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetMetaagentTurnBlobRequest {
    pub session_id: String,
    pub metaagent_id: String,
    pub blob_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadMetaagentEventRequest {
    pub session_id: String,
    pub metaagent_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AckMetaagentEventsRequest {
    pub session_id: String,
    pub metaagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_to_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    GetWorkspaceLiveSyncStatus(GetWorkspaceLiveSyncStatusRequest),
    LaunchProviderRun(LaunchProviderRunRequest),
    ListSessions(ListSessionsRequest),
    ResolveSession(ResolveSessionRequest),
    GetSessionState(GetSessionStateRequest),
    UpdateMetaagentTask(UpdateMetaagentTaskRequest),
    PauseMetaagentTask(PauseMetaagentTaskRequest),
    ResumeMetaagentTask(ResumeMetaagentTaskRequest),
    AbortMetaagentTask(AbortMetaagentTaskRequest),
    SearchMetaagentCommands(SearchMetaagentCommandsRequest),
    GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest),
    GetMetaagentTurnBlob(GetMetaagentTurnBlobRequest),
    ListMetaagentEvents(ListMetaagentEventsRequest),
    ReadMetaagentEvent(ReadMetaagentEventRequest),
    AckMetaagentEvents(AckMetaagentEventsRequest),
    GetDaemonHealth(GetDaemonHealthRequest),
    ExportDebugBundle(ExportDebugBundleRequest),
    GetProviderRun(GetProviderRunRequest),
    UpdateProviderRunSelection(UpdateProviderRunSelectionRequest),
    GetProviderCatalog(GetProviderCatalogRequest),
    GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest),
    InstallMcpServer(InstallMcpServerRequest),
    UpdateMcpServer(UpdateMcpServerRequest),
    UninstallMcpServer(UninstallMcpServerRequest),
    ImportMcpServers(ImportMcpServersRequest),
    ImportProviderCapabilities(ImportProviderCapabilitiesRequest),
    GetMcpServer(GetMcpServerRequest),
    ListMcpServers(ListMcpServersRequest),
    RegisterEnvironment(RegisterEnvironmentRequest),
    RemoveEnvironment(RemoveEnvironmentRequest),
    GetEnvironment(GetEnvironmentRequest),
    ListEnvironments(ListEnvironmentsRequest),
    ValidateScript(ValidateScriptRequest),
    RegisterScript(RegisterScriptRequest),
    RemoveScript(RemoveScriptRequest),
    GetScript(GetScriptRequest),
    ListScripts(ListScriptsRequest),
    RegisterCredential(RegisterCredentialRequest),
    UpsertCredential(UpsertCredentialRequest),
    RemoveCredential(RemoveCredentialRequest),
    GetCredential(GetCredentialRequest),
    ListCredentials(ListCredentialsRequest),
    RegisterConnector(RegisterConnectorRequest),
    UpsertConnector(UpsertConnectorRequest),
    RegisterConnectorAdapter(RegisterConnectorAdapterRequest),
    RemoveConnectorAdapter(RemoveConnectorAdapterRequest),
    GetConnectorAdapter(GetConnectorAdapterRequest),
    ListConnectorAdapters(ListConnectorAdaptersRequest),
    RemoveConnector(RemoveConnectorRequest),
    GetConnector(GetConnectorRequest),
    ListConnectors(ListConnectorsRequest),
    TestConnector(TestConnectorRequest),
    UpsertSkill(UpsertSkillRequest),
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
    ResolveKernelClientConnection(ResolveKernelClientConnectionRequest),
    CreateCloudSessionInvite(CreateCloudSessionInviteRequest),
    ShowCloudSessionInvite(ShowCloudSessionInviteRequest),
    AcceptCloudSessionInvite(AcceptCloudSessionInviteRequest),
    RevokeCloudSessionInvite(RevokeCloudSessionInviteRequest),
    ListCloudSessionMembers(ListCloudSessionMembersRequest),
    ListCloudCollaborators(ListCloudCollaboratorsRequest),
    GetUserConfig(GetUserConfigRequest),
    GetUserConfigSchema(GetUserConfigSchemaRequest),
    SetUserConfigValue(SetUserConfigValueRequest),
    SetWorkspaceLiveSyncMode(SetWorkspaceLiveSyncModeRequest),
    UnsetUserConfigValue(UnsetUserConfigValueRequest),
    SetCredentialSecret(SetCredentialSecretRequest),
    DeleteCredentialSecret(DeleteCredentialSecretRequest),
    GetCredentialVaultStatus(GetCredentialVaultStatusRequest),
    LockCredentialVault(LockCredentialVaultRequest),
    ManageCredentialVault(ManageCredentialVaultRequest),
    ListSlices(ListSlicesRequest),
    CreateSlice(CreateSliceRequest),
    GetSlice(SliceRefRequest),
    StartSlice(SliceRefRequest),
    StopSlice(SliceRefRequest),
    DeleteSlice(SliceRefRequest),
    ImportSliceProviderAuth(ImportSliceProviderAuthRequest),
    RemoveSliceProviderAuth(RemoveSliceProviderAuthRequest),
    StartSliceProviderLogin(StartSliceProviderLoginRequest),
    SetSliceProviderAuthAlias(SetSliceProviderAuthAliasRequest),
    GetSliceDisplayEndpoint(SliceRefRequest),
    GetSliceLogs(GetSliceLogsRequest),
    ListSliceAudit(ListSliceAuditRequest),
    SaveSliceState(SliceStateSaveRequest),
    GetSliceStateStatus(SliceStateStatusRequest),
    ResetSliceState(SliceStateResetRequest),
    CreateSliceBackup(CreateSliceBackupRequest),
    ListRemoteMachines(ListRemoteMachinesRequest),
    ListRemoteMachineKernels(ListRemoteMachineKernelsRequest),
    GetWaitingRoomInventory(GetWaitingRoomInventoryRequest),
    GetWaitingRoomPublicSnapshot(GetWaitingRoomPublicSnapshotRequest),
    ListExternalProviderSessions(ListExternalProviderSessionsRequest),
    RefreshExternalProviderSessions(RefreshExternalProviderSessionsRequest),
    ImportExternalProviderSession(ImportExternalProviderSessionRequest),
    ImportExternalProviderAgent(ImportExternalProviderAgentRequest),
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
    GetSessionHistoryOutline(GetSessionHistoryOutlineRequest),
    GetSessionHistoryBlobContent(GetSessionHistoryBlobContentRequest),
    GetPromptInputHistory(GetPromptInputHistoryRequest),
    RecordPromptInputHistory(RecordPromptInputHistoryRequest),
    QueryRecall(QueryRecallRequest),
    SearchRecall(SearchRecallRequest),
    SemanticSearchRecall(SemanticSearchRecallRequest),
    PollRuntimeNotices(PollRuntimeNoticesRequest),
    RespondToInteraction(RespondToInteractionRequest),
    RequestNativeProviderInteraction(RequestNativeProviderInteractionRequest),
    SubmitPrompt(SubmitPromptRequest),
    CompletePrompt(CompletePromptRequest),
    CancelActivePrompt(CancelActivePromptRequest),
    SteerQueuedPrompt(SteerQueuedPromptRequest),
    CancelQueuedPrompt(CancelQueuedPromptRequest),
    UpdateSessionConfig(UpdateSessionConfigRequest),
    UpdateAgentConfig(UpdateAgentConfigRequest),
    UpdateAgentProfile(UpdateAgentProfileRequest),
    UpdateAgentSubstitutes(UpdateAgentSubstitutesRequest),
    ResizeTerminal(ResizeTerminalRequest),
    SendTerminalInput(SendTerminalInputRequest),
    PumpTerminalOutput(PumpTerminalOutputRequest),
    AppendNativeProviderOutput(AppendNativeProviderOutputRequest),
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
    UndoTurn(UndoTurnRequest),
    ForkAgent(ForkAgentRequest),
    MoveAgentToRemote(MoveAgentToRemoteRequest),
    MoveAgentToLocal(MoveAgentToLocalRequest),
    SyncRemoteExtensionManifest(SyncRemoteExtensionManifestRequest),
    ListHomeExtensionAudit(ListHomeExtensionAuditRequest),
    DestroyAgent(DestroyAgentRequest),
    FocusAgent(FocusAgentRequest),
    AcknowledgeAgentOutputSeen(AcknowledgeAgentOutputSeenRequest),
    CycleAgentFocus(CycleAgentFocusRequest),
    GrantAgentExtension(GrantAgentExtensionRequest),
    RevokeAgentExtension(RevokeAgentExtensionRequest),
    ListAgents(ListAgentsRequest),
    CreateWorkflow(CreateWorkflowRequest),
    ValidateWorkflowCode(ValidateWorkflowCodeRequest),
    ApplyWorkflowCode(ApplyWorkflowCodeRequest),
    ApplyWorkflowCodeArtifact(ApplyWorkflowCodeArtifactRequest),
    RunWorkflowCode(RunWorkflowCodeRequest),
    RunWorkflowCodeArtifact(RunWorkflowCodeArtifactRequest),
    ListWorkflowRegistry(ListWorkflowRegistryRequest),
    GetWorkflowRegistryEntry(GetWorkflowRegistryEntryRequest),
    AddWorkflowRegistryEntry(AddWorkflowRegistryEntryRequest),
    AddWorkflowRegistryEntryFromWorkflow(AddWorkflowRegistryEntryFromWorkflowRequest),
    DeleteWorkflowRegistryEntry(DeleteWorkflowRegistryEntryRequest),
    LoadWorkflowRegistryEntry(LoadWorkflowRegistryEntryRequest),
    RunWorkflowRegistryEntry(RunWorkflowRegistryEntryRequest),
    CreateWorkflowCodeArtifact(CreateWorkflowCodeArtifactRequest),
    UpdateWorkflowCodeArtifact(UpdateWorkflowCodeArtifactRequest),
    GetWorkflowCodeArtifact(GetWorkflowCodeArtifactRequest),
    ListWorkflowCodeArtifacts(ListWorkflowCodeArtifactsRequest),
    DeleteWorkflowCodeArtifact(DeleteWorkflowCodeArtifactRequest),
    ExportWorkflowCodeArtifact(ExportWorkflowCodeArtifactRequest),
    ImportWorkflowCodeArtifact(ImportWorkflowCodeArtifactRequest),
    ExportWorkflowCodePackage(ExportWorkflowCodePackageRequest),
    ImportWorkflowCodePackage(ImportWorkflowCodePackageRequest),
    ExportWorkflowCodeSource(ExportWorkflowCodeSourceRequest),
    ApplyWorkflowDesignOp(ApplyWorkflowDesignOpRequest),
    AliasWorkflow(AliasWorkflowRequest),
    ListWorkflows(ListWorkflowsRequest),
    ResolveWorkflow(ResolveWorkflowRequest),
    CreateWorkflowPublication(CreateWorkflowPublicationRequest),
    ListWorkflowPublications(ListWorkflowPublicationsRequest),
    GetWorkflowPublication(GetWorkflowPublicationRequest),
    ExportWorkflowPublicationPackage(ExportWorkflowPublicationPackageRequest),
    DisableWorkflowPublication(DisableWorkflowPublicationRequest),
    RegisterWorkflowPublicationEndpoint(RegisterWorkflowPublicationEndpointRequest),
    MaterializeWorkflowPublication(MaterializeWorkflowPublicationRequest),
    CreateWorkflowEndpoint(CreateWorkflowEndpointRequest),
    AliasWorkflowEndpoint(AliasWorkflowEndpointRequest),
    BindWorkflowEndpoint(BindWorkflowEndpointRequest),
    AddWorkflowNode(AddWorkflowNodeRequest),
    RemoveWorkflowNode(RemoveWorkflowNodeRequest),
    UpdateWorkflowNodeInstructions(UpdateWorkflowNodeInstructionsRequest),
    SetWorkflowNodeCanCompleteRun(SetWorkflowNodeCanCompleteRunRequest),
    SetWorkflowNodeCanEmitIntermediateOutput(SetWorkflowNodeCanEmitIntermediateOutputRequest),
    SetWorkflowNodeWaitForAllInputs(SetWorkflowNodeWaitForAllInputsRequest),
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
    ListWorkflowPromptQueues(ListWorkflowPromptQueuesRequest),
    CreateWorkflowPromptQueue(CreateWorkflowPromptQueueRequest),
    UpdateWorkflowPromptQueue(UpdateWorkflowPromptQueueRequest),
    RemoveWorkflowPromptQueue(RemoveWorkflowPromptQueueRequest),
    ListQueuedWorkflowPrompts(ListQueuedWorkflowPromptsRequest),
    UpdateQueuedWorkflowPrompt(UpdateQueuedWorkflowPromptRequest),
    RemoveQueuedWorkflowPrompt(RemoveQueuedWorkflowPromptRequest),
    ClearWorkflowPromptQueue(ClearWorkflowPromptQueueRequest),
    ValidateWorkflowHandoff(ValidateWorkflowHandoffRequest),
    AckWorkflowTurn(AckWorkflowTurnRequest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    WorkspaceLiveSyncStatus {
        status: WorkspaceLiveSyncStatus,
    },
    WorkspaceLiveSyncModeUpdated {
        session: RuntimeSession,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<UserConfigMutationEffect>,
    },
    ProviderRunLaunched {
        provider_run: RuntimeProviderRun,
    },
    NativeProviderInteractionResolved {
        resolution: NativeProviderInteractionResolution,
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
        #[serde(default)]
        agent_activity_revision: u64,
    },
    MetaagentTaskUpdated {
        session: RuntimeSession,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<crate::session::MetaagentTask>,
    },
    MetaagentEventsListed {
        events: Vec<serde_json::Value>,
    },
    MetaagentCommandsSearched {
        commands: Vec<serde_json::Value>,
    },
    MetaagentTurnOverview {
        overview: serde_json::Value,
    },
    MetaagentTurnBlob {
        blob: serde_json::Value,
    },
    MetaagentEventRead {
        event: serde_json::Value,
    },
    MetaagentEventsAcked {
        acked: Vec<serde_json::Value>,
    },
    DaemonHealth {
        projection: DaemonHealthProjection,
    },
    DebugBundleExported {
        bundle_dir: String,
        manifest_path: String,
        logs_path: String,
        log_root: String,
        record_count: usize,
        limit: usize,
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
    ProviderCapabilitiesImported {
        report: ProviderCapabilityImportReport,
    },
    McpServer {
        mcp: ArrobaMcpServerConfig,
    },
    McpServersListed {
        mcps: Vec<ArrobaMcpServerConfig>,
    },
    EnvironmentRegistered {
        environment: ArrobaEnvironmentConfig,
        path: PathBuf,
    },
    EnvironmentRemoved {
        name: String,
        path: PathBuf,
    },
    Environment {
        environment: ArrobaEnvironmentConfig,
    },
    EnvironmentsListed {
        environments: Vec<ArrobaEnvironmentConfig>,
    },
    ScriptValidated {
        script: ArrobaScriptMetadata,
    },
    ScriptRegistered {
        script: ArrobaScriptMetadata,
        path: PathBuf,
    },
    ScriptRemoved {
        script: ArrobaScriptMetadata,
        path: PathBuf,
    },
    Script {
        script: ArrobaScriptMetadata,
    },
    ScriptsListed {
        scripts: Vec<ArrobaScriptMetadata>,
    },
    CredentialRegistered {
        credential: UserCredentialConfig,
        path: PathBuf,
    },
    CredentialUpserted {
        credential: UserCredentialConfig,
        path: PathBuf,
    },
    CredentialRemoved {
        credential: UserCredentialConfig,
        path: PathBuf,
    },
    Credential {
        credential: UserCredentialConfig,
    },
    CredentialsListed {
        credentials: Vec<UserCredentialConfig>,
    },
    ConnectorRegistered {
        connector: ArrobaConnectorDefinition,
        path: PathBuf,
    },
    ConnectorUpserted {
        connector: ArrobaConnectorDefinition,
        path: PathBuf,
    },
    ConnectorAdapterRegistered {
        adapter: ArrobaConnectorAdapterDefinition,
        path: PathBuf,
    },
    ConnectorAdapterRemoved {
        adapter: ArrobaConnectorAdapterDefinition,
        path: PathBuf,
    },
    ConnectorAdapter {
        adapter: ArrobaConnectorAdapterDefinition,
    },
    ConnectorAdaptersListed {
        adapters: Vec<ArrobaConnectorAdapterDefinition>,
    },
    ConnectorRemoved {
        connector: ArrobaConnectorDefinition,
        path: PathBuf,
    },
    Connector {
        connector: ArrobaConnectorDefinition,
    },
    ConnectorsListed {
        connectors: Vec<ArrobaConnectorDefinition>,
    },
    ConnectorTested {
        execution: ConnectorExecution,
    },
    Skill {
        skill: ArrobaSkillMetadata,
    },
    SkillInstalled {
        skill: ArrobaSkillMetadata,
        path: PathBuf,
    },
    SkillUpserted {
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
    KernelClientConnectionResolved {
        connection: KernelClientConnection,
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
    CredentialVaultStatus {
        status: crate::secret::ArrobaVaultUnlockStatus,
    },
    CredentialVaultLocked {
        status: crate::secret::ArrobaVaultUnlockStatus,
    },
    CredentialVaultManaged {
        status: crate::secret::ArrobaVaultUnlockStatus,
        action: String,
    },
    SlicesListed {
        slices: Vec<SliceRecord>,
    },
    SliceCreated {
        slice: SliceRecord,
    },
    Slice {
        slice: SliceRecord,
    },
    SliceStarted {
        slice: SliceRecord,
    },
    SliceStopped {
        slice: SliceRecord,
    },
    SliceDeleted {
        slice: SliceRecord,
    },
    SliceProviderAuthImported {
        slice: SliceRecord,
        provider: String,
        status: String,
    },
    SliceProviderAuthRemoved {
        slice: SliceRecord,
        provider: String,
        status: String,
    },
    SliceProviderLoginStarted {
        slice: SliceRecord,
        login: SliceProviderLoginStart,
    },
    SliceProviderAuthAliasSet {
        slice: SliceRecord,
        provider: String,
        alias: Option<String>,
    },
    SliceDisplayEndpoint {
        endpoint: SliceDisplayEndpoint,
    },
    SliceLogs {
        slice: SliceRecord,
        entries: Vec<SliceLogEntry>,
    },
    SliceStateSaved {
        slice: SliceRecord,
        state: crate::slice::SliceSavedStateRecord,
    },
    SliceStateStatus {
        slice: SliceRecord,
        state: Option<crate::slice::SliceSavedStateRecord>,
    },
    SliceStateReset {
        slice: SliceRecord,
        removed_state: Option<crate::slice::SliceSavedStateRecord>,
    },
    SliceBackupCreated {
        slice: SliceRecord,
        backup: crate::slice::SliceBackupRecord,
        instructions: String,
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
    ExternalProviderSessionsListed {
        page: ExternalProviderSessionPage,
    },
    ExternalProviderSessionsRefreshed {
        page: ExternalProviderSessionPage,
    },
    ExternalProviderSessionImported {
        session: RuntimeSession,
        agent: AgentInstance,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_run: Option<RuntimeProviderRun>,
    },
    ExternalProviderAgentImported {
        session: RuntimeSession,
        agent: AgentInstance,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_run: Option<RuntimeProviderRun>,
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
    SessionHistoryOutline {
        agents: Vec<SessionHistoryOutlineAgent>,
    },
    SessionHistoryBlobContent {
        blob_id: String,
        entries: Vec<SessionHistoryPageEntry>,
    },
    PromptInputHistory {
        entries: Vec<PromptInputHistoryEntry>,
    },
    PromptInputHistoryRecorded {
        entry: PromptInputHistoryEntry,
    },
    RecallEvents {
        events: Vec<HistoryEvent>,
        next_sequence: Option<u64>,
    },
    SemanticRecallEvents {
        results: Vec<SemanticRecallMatch>,
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
        #[serde(default)]
        agent_activity_revision: u64,
    },
    PromptCompleted {
        completion: PromptCompletion,
    },
    PromptCancelled {
        cancellation: PromptCancellation,
    },
    QueuedPromptSteered {
        prompt: PromptQueueItem,
        session: RuntimeSession,
        agent_activity: BTreeMap<String, crate::runtime::projection::AgentRuntimeActivity>,
        #[serde(default)]
        agent_activity_revision: u64,
    },
    QueuedPromptCancelled {
        prompt: PromptQueueItem,
        session: RuntimeSession,
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
    TerminalInputSent {
        session_id: String,
        attachment_id: String,
        byte_count: usize,
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
    TurnUndone {
        result: TurnUndoResult,
    },
    AgentForked {
        source_agent_id: String,
        agent: AgentInstance,
        provider_run: RuntimeProviderRun,
        session: RuntimeSession,
    },
    AgentMovedToRemote {
        agent: AgentInstance,
    },
    AgentMovedToLocal {
        agent: AgentInstance,
    },
    RemoteExtensionManifestSynced {
        agent: AgentInstance,
    },
    HomeExtensionAuditListed {
        events: Vec<crate::durable_state::DurableStateEvent>,
    },
    SliceAuditListed {
        events: Vec<crate::durable_state::DurableStateEvent>,
    },
    AgentDestroyed {
        agent: AgentInstance,
    },
    AgentFocused {
        agent: AgentInstance,
    },
    AgentOutputSeenAcknowledged {
        session_id: String,
        agent_id: String,
    },
    AgentFocusCycled {
        agent: Option<AgentInstance>,
    },
    AgentExtensionGranted {
        agent: AgentInstance,
    },
    AgentExtensionRevoked {
        agent: AgentInstance,
    },
    AgentsListed {
        agents: Vec<AgentInstance>,
    },
    WorkflowCreated {
        workflow: WorkflowDefinition,
        session: RuntimeSession,
    },
    WorkflowCodeValidated {
        result: crate::workflow_code::WorkflowCodeCompileResult,
    },
    WorkflowCodeApplied {
        result: crate::workflow_code::WorkflowCodeCompileAndApplyResult,
        session: RuntimeSession,
    },
    WorkflowCodeRun {
        result: crate::workflow_code::WorkflowCodeRunResult,
        session: RuntimeSession,
    },
    WorkflowRegistryListed {
        entries: Vec<crate::workflow_code::WorkflowRegistryEntryMetadata>,
    },
    WorkflowRegistryEntry {
        entry: crate::workflow_code::WorkflowRegistryEntryMetadata,
    },
    WorkflowRegistryEntryAdded {
        entry: crate::workflow_code::WorkflowRegistryEntryMetadata,
    },
    WorkflowRegistryEntryDeleted {
        name: String,
        path: PathBuf,
    },
    WorkflowRegistryEntryLoaded {
        entry: crate::workflow_code::WorkflowRegistryEntryMetadata,
        result: crate::workflow_code::WorkflowCodeCompileAndApplyResult,
        session: RuntimeSession,
    },
    WorkflowRegistryEntryRun {
        entry: crate::workflow_code::WorkflowRegistryEntryMetadata,
        result: crate::workflow_code::WorkflowCodeRunResult,
        session: RuntimeSession,
    },
    WorkflowCodeArtifactCreated {
        artifact: crate::workflow_code::WorkflowCodeArtifact,
    },
    WorkflowCodeArtifactUpdated {
        artifact: crate::workflow_code::WorkflowCodeArtifact,
    },
    WorkflowCodeArtifact {
        artifact: crate::workflow_code::WorkflowCodeArtifact,
    },
    WorkflowCodeArtifactsListed {
        artifacts: Vec<crate::workflow_code::WorkflowCodeArtifactMetadata>,
    },
    WorkflowCodeArtifactDeleted {
        name: String,
        path: PathBuf,
    },
    WorkflowCodeArtifactExported {
        package: crate::workflow_code::WorkflowCodeArtifactPackage,
    },
    WorkflowCodeArtifactImported {
        artifact: crate::workflow_code::WorkflowCodeArtifact,
    },
    WorkflowCodePackageExported {
        package: crate::workflow_code::WorkflowCodeArtifactPackage,
    },
    WorkflowCodePackageImported {
        artifact: crate::workflow_code::WorkflowCodeArtifact,
    },
    WorkflowCodeSourceExported {
        export: crate::workflow_code::WorkflowCodeSourceExport,
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
    WorkflowPublicationPackageExported {
        publication: WorkflowPublicationDefinition,
        package_version: u32,
        package_digest: String,
        package_archive_base64: String,
        package_files: Vec<WorkflowPublicationPackageFile>,
    },
    WorkflowPublicationDisabled {
        publication: WorkflowPublicationDefinition,
        session: RuntimeSession,
    },
    WorkflowPublicationEndpointRegistered {
        publication: WorkflowPublicationDefinition,
        open_url: String,
        access: String,
        expires_at_ms: Option<u64>,
    },
    WorkflowPublicationMaterialized {
        publication_id: String,
        session: RuntimeSession,
        agent_id_map: BTreeMap<String, String>,
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
    WorkflowNodeWaitForAllInputsUpdated {
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
    WorkflowPromptEnqueued {
        queued_prompt: WorkflowQueuedPrompt,
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
    WorkflowPromptQueuesListed {
        queues: Vec<WorkflowPromptQueueDefinition>,
    },
    WorkflowPromptQueueCreated {
        queue: WorkflowPromptQueueDefinition,
        session: RuntimeSession,
    },
    WorkflowPromptQueueUpdated {
        queue: WorkflowPromptQueueDefinition,
        session: RuntimeSession,
    },
    WorkflowPromptQueueRemoved {
        queue: WorkflowPromptQueueDefinition,
        session: RuntimeSession,
    },
    QueuedWorkflowPromptsListed {
        queued_prompts: Vec<WorkflowQueuedPrompt>,
    },
    QueuedWorkflowPromptUpdated {
        queued_prompt: WorkflowQueuedPrompt,
        session: RuntimeSession,
    },
    QueuedWorkflowPromptRemoved {
        queued_prompt: WorkflowQueuedPrompt,
        session: RuntimeSession,
    },
    WorkflowPromptQueueCleared {
        queued_prompts: Vec<WorkflowQueuedPrompt>,
        session: RuntimeSession,
    },
    WorkflowHandoffValidated {
        valid: bool,
        warning: Option<String>,
    },
    WorkflowTurnAcknowledged {
        workflow_run: WorkflowRun,
        session: RuntimeSession,
    },
}

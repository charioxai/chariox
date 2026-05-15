import type {
  HistoryQueryPayload,
  PromptAttachmentPart,
  SemanticSearchHistoryMode,
  SessionAgentDefaults,
  SessionHistoryCursor,
} from "./kernel-types.js"

export * from "./ipc-workflow-requests.js"
export * from "./ipc-workspace-requests.js"
export * from "./ipc-remote-connection-requests.js"

export function createSessionRequest(
  workspaceId: string,
  worktreeId: string,
  alias?: string,
  agentDefaults?: SessionAgentDefaults,
  sliceRef?: string | null,
) {
  return {
    CreateSession: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      alias: alias ?? null,
      ...(agentDefaults ? { agent_defaults: agentDefaults } : {}),
      slice_ref: sliceRef ?? null,
    },
  }
}

export function listSessionsRequest() {
  return { ListSessions: null }
}

export function resolveSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    ResolveSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

export function attachToSessionRequest(sessionId: string, clientId: string) {
  return {
    AttachToSession: {
      session_id: sessionId,
      client_id: clientId,
      capability_level: "FullTerminal",
    },
  }
}

export function detachFromSessionRequest(attachmentId: string) {
  return {
    DetachFromSession: {
      attachment_id: attachmentId,
    },
  }
}

export function listSessionMembersRequest(sessionId: string) {
  return {
    ListSessionMembers: {
      session_id: sessionId,
    },
  }
}

export function createSessionInviteRequest(
  sessionId: string,
  expiresInMs: number | null = null,
  maxUses: number | null = null,
) {
  return {
    CreateSessionInvite: {
      session_id: sessionId,
      expires_in_ms: expiresInMs,
      max_uses: maxUses,
    },
  }
}

export function joinSessionInviteRequest(inviteToken: string, userId: string) {
  return {
    JoinSessionInvite: {
      invite_token: inviteToken,
      user_id: userId,
    },
  }
}

export function revokeSessionInviteRequest(sessionId: string, inviteRef: string) {
  return {
    RevokeSessionInvite: {
      session_id: sessionId,
      invite_ref: inviteRef,
    },
  }
}

export function createWorkspaceLinkRequest(sessionId: string, name: string) {
  return {
    CreateWorkspaceLink: {
      session_id: sessionId,
      name,
    },
  }
}

export function listWorkspaceLinksRequest(sessionId: string) {
  return {
    ListWorkspaceLinks: {
      session_id: sessionId,
    },
  }
}

export function showWorkspaceLinkRequest(sessionId: string, linkRef: string) {
  return {
    ShowWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
    },
  }
}

export function attachWorkspaceLinkRequest(
  sessionId: string,
  linkRef: string,
  repoRoot?: string | null,
  branch?: string | null,
  repoFingerprint?: string | null,
) {
  return {
    AttachWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
      repo_root: repoRoot ?? null,
      branch: branch ?? null,
      repo_fingerprint: repoFingerprint ?? null,
    },
  }
}

export function detachWorkspaceLinkRequest(sessionId: string, linkRef: string, repoRoot?: string | null) {
  return {
    DetachWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
      repo_root: repoRoot ?? null,
    },
  }
}

export function endSessionRequest(sessionId: string) {
  return {
    EndSession: {
      session_id: sessionId,
    },
  }
}

export function deleteSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    DeleteSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

export function deleteKernelRequest() {
  return { DeleteKernel: null }
}

export function aliasSessionRequest(sessionId: string, alias: string) {
  return {
    AliasSession: {
      session_id: sessionId,
      alias,
    },
  }
}

export function aliasAgentRequest(sessionId: string, agentId: string, alias: string) {
  return {
    AliasAgent: {
      session_id: sessionId,
      agent_id: agentId,
      alias,
    },
  }
}

export function getSessionStateRequest(sessionId: string) {
  return {
    GetSessionState: {
      session_id: sessionId,
    },
  }
}

export function getDaemonHealthRequest() {
  return { GetDaemonHealth: null }
}


export function installMcpServerRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    InstallMcpServer: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function updateMcpServerRequest(workspaceId: string | null, config: Record<string, unknown>) {
  return {
    UpdateMcpServer: {
      workspace_id: workspaceId ?? null,
      config,
    },
  }
}

export function uninstallMcpServerRequest(workspaceId: string | null, name: string) {
  return {
    UninstallMcpServer: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function getMcpServerRequest(workspaceId: string | null, name: string) {
  return {
    GetMcpServer: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function importMcpServersRequest(workspaceId: string | null, provider: string, name?: string | null) {
  return {
    ImportMcpServers: {
      workspace_id: workspaceId ?? null,
      provider,
      name: name ?? null,
    },
  }
}

export function getSkillRequest(workspaceId: string | null, name: string) {
  return {
    GetSkill: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function installSkillRequest(workspaceId: string | null, sourcePath: string) {
  return {
    InstallSkill: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
    },
  }
}

export function updateSkillRequest(workspaceId: string | null, sourcePath: string) {
  return {
    UpdateSkill: {
      workspace_id: workspaceId ?? null,
      source_path: sourcePath,
    },
  }
}

export function uninstallSkillRequest(workspaceId: string | null, name: string) {
  return {
    UninstallSkill: {
      workspace_id: workspaceId ?? null,
      name,
    },
  }
}

export function importSkillsRequest(workspaceId: string | null, provider: string, name?: string | null) {
  return {
    ImportSkills: {
      workspace_id: workspaceId ?? null,
      provider,
      name: name ?? null,
    },
  }
}

export function grantAgentCapabilityRequest(
  workspaceId: string | null,
  agentRef: string,
  kind: "mcp" | "skill",
  name: string,
) {
  return {
    GrantAgentCapability: {
      workspace_id: workspaceId ?? null,
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function revokeAgentCapabilityRequest(agentRef: string, kind: "mcp" | "skill", name: string) {
  return {
    RevokeAgentCapability: {
      agent_ref: agentRef,
      kind,
      name,
    },
  }
}

export function listMcpServersRequest(workspaceId?: string | null) {
  return {
    ListMcpServers: {
      workspace_id: workspaceId ?? null,
    },
  }
}

export function listSkillsRequest(workspaceId?: string | null) {
  return {
    ListSkills: {
      workspace_id: workspaceId ?? null,
    },
  }
}

export function listProviderProcessesRequest(provider?: string | null) {
  return {
    ListProviderProcesses: {
      provider: provider ?? null,
    },
  }
}

export function teardownProviderProcessesRequest(provider?: string | null, force = false) {
  return {
    TeardownProviderProcesses: {
      provider: provider ?? null,
      force,
    },
  }
}

export function updateSessionConfigRequest(
  sessionId: string,
  attachmentId: string,
  values: Record<string, string>,
  requiresIdle = false,
) {
  return {
    UpdateSessionConfig: {
      session_id: sessionId,
      attachment_id: attachmentId,
      values,
      requires_idle: requiresIdle,
    },
  }
}

export function getProviderRunRequest(providerRunId: string) {
  return {
    GetProviderRun: {
      provider_run_id: providerRunId,
    },
  }
}

export function updateProviderRunSelectionRequest(
  sessionId: string,
  providerRunId: string,
  options: { model?: string | null; variant?: string | null; clearVariant?: boolean } = {},
) {
  return {
    UpdateProviderRunSelection: {
      session_id: sessionId,
      provider_run_id: providerRunId,
      model: options.model ?? null,
      variant: options.variant ?? null,
      clear_variant: options.clearVariant ?? false,
    },
  }
}

export function getProviderCatalogRequest() {
  return { GetProviderCatalog: null }
}

export function relayStatusRequest() {
  return { RelayStatus: null }
}

export function configureRelayRequest(relayUrl: string | null, relayToken: string | null) {
  return {
    ConfigureRelay: {
      relay_url: relayUrl,
      relay_token: relayToken,
    },
  }
}

export function cloudRelayStatusRequest() {
  return { CloudRelayStatus: null }
}

export function startCloudRelayLoginRequest(apiUrl: string, input: {
  clientId?: string
  clientAlias?: string
  machineId?: string
  machineAlias?: string
}) {
  return {
    StartCloudRelayLogin: {
      api_url: apiUrl,
      client_id: input.clientId,
      client_alias: input.clientAlias,
      machine_id: input.machineId,
      machine_alias: input.machineAlias,
    },
  }
}

export function pollCloudRelayLoginRequest(apiUrl: string, deviceCode: string) {
  return {
    PollCloudRelayLogin: {
      api_url: apiUrl,
      device_code: deviceCode,
    },
  }
}

export function logoutCloudRelayRequest(options: { revokeClient?: boolean; revokeMachine?: boolean } = {}) {
  return {
    LogoutCloudRelay: {
      revoke_client: options.revokeClient ?? false,
      revoke_machine: options.revokeMachine ?? false,
    },
  }
}

export function pairCloudRelayClientRequest(clientId: string, alias?: string) {
  return {
    PairCloudRelayClient: {
      client_id: clientId,
      alias,
    },
  }
}

export function pairCloudRelayMachineRequest(machineId: string, alias?: string) {
  return {
    PairCloudRelayMachine: {
      machine_id: machineId,
      alias,
    },
  }
}

export function connectCloudRelayRequest() {
  return { ConnectCloudRelay: null }
}

export function issueCloudRelayClientTokenRequest(targetDaemonAlias: string, clientId: string, sessionId?: string | null) {
  return {
    IssueCloudRelayClientToken: {
      target_daemon_alias: targetDaemonAlias,
      client_id: clientId,
      session_id: sessionId ?? null,
    },
  }
}

export function createCloudSessionInviteRequest(
  sessionId: string,
  options: { displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null } = {},
) {
  return {
    CreateCloudSessionInvite: {
      session_id: sessionId,
      display_name: options.displayName ?? null,
      expires_in_ms: options.expiresInMs ?? null,
      max_uses: options.maxUses ?? null,
    },
  }
}

export function showCloudSessionInviteRequest(inviteToken: string) {
  return {
    ShowCloudSessionInvite: {
      invite_token: inviteToken,
    },
  }
}

export function acceptCloudSessionInviteRequest(inviteToken: string) {
  return {
    AcceptCloudSessionInvite: {
      invite_token: inviteToken,
    },
  }
}

export function revokeCloudSessionInviteRequest(sessionId: string, inviteId: string) {
  return {
    RevokeCloudSessionInvite: {
      session_id: sessionId,
      invite_id: inviteId,
    },
  }
}

export function listCloudSessionMembersRequest(sessionId: string) {
  return {
    ListCloudSessionMembers: {
      session_id: sessionId,
    },
  }
}

export function listCloudCollaboratorsRequest() {
  return { ListCloudCollaborators: null }
}

export function getUserConfigRequest() {
  return { GetUserConfig: null }
}

export function getUserConfigSchemaRequest() {
  return { GetUserConfigSchema: null }
}

export function setUserConfigValueRequest(path: string, value: string) {
  return {
    SetUserConfigValue: {
      path,
      value,
    },
  }
}

export function unsetUserConfigValueRequest(path: string) {
  return {
    UnsetUserConfigValue: {
      path,
    },
  }
}

export function setCredentialSecretRequest(key: string, value: string) {
  return {
    SetCredentialSecret: {
      key,
      value,
    },
  }
}

export function deleteCredentialSecretRequest(key: string) {
  return {
    DeleteCredentialSecret: {
      key,
    },
  }
}

export function getProviderCommandCatalogsRequest() {
  return { GetProviderCommandCatalogs: null }
}

export function getProviderAuthStatusRequest(provider: string) {
  return {
    GetProviderAuthStatus: {
      provider,
    },
  }
}

export function startProviderLoginRequest(provider: string) {
  return {
    StartProviderLogin: {
      provider,
    },
  }
}

export function logoutProviderRequest(provider: string) {
  return {
    LogoutProvider: {
      provider,
    },
  }
}

export function readDirectoryTreeRequest(sessionId: string, attachmentId: string, treePath: string | null, maxDepth: number) {
  return {
    ReadDirectoryTree: {
      session_id: sessionId,
      attachment_id: attachmentId,
      path: treePath,
      max_depth: maxDepth,
    },
  }
}

export function getSessionHistoryRequest(
  sessionId: string,
  roundCount: number,
  maxChars: number,
  cursor?: SessionHistoryCursor | null,
  agentId?: string | null,
) {
  return {
    GetSessionHistory: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      round_count: roundCount,
      max_chars: maxChars,
      before_entry_index: cursor?.before_entry_index ?? null,
      before_entry_char_offset: cursor?.before_entry_char_offset ?? null,
    },
  }
}

export function getPromptInputHistoryRequest(
  sessionId: string,
  afterSequence?: number | null,
  limit?: number | null,
) {
  return {
    GetPromptInputHistory: {
      session_id: sessionId,
      after_sequence: afterSequence ?? null,
      limit: limit ?? null,
    },
  }
}

export function recordPromptInputHistoryRequest(
  sessionId: string,
  attachmentId: string | null,
  kind: "prompt" | "command",
  text: string,
) {
  return {
    RecordPromptInputHistory: {
      session_id: sessionId,
      attachment_id: attachmentId,
      kind,
      text,
    },
  }
}

export function queryHistoryRequest(query: HistoryQueryPayload) {
  return {
    QueryHistory: {
      session_id: query.session_id ?? null,
      agent_id: query.agent_id ?? null,
      provider: query.provider ?? null,
      model: query.model ?? null,
      workflow_id: query.workflow_id ?? null,
      machine_id: query.machine_id ?? null,
      repo_root: query.repo_root ?? null,
      worktree_path: query.worktree_path ?? null,
      kind: query.kind ?? null,
      text: query.text ?? null,
      after_sequence: query.after_sequence ?? null,
      before_sequence: query.before_sequence ?? null,
      limit: query.limit ?? null,
    },
  }
}

export function searchHistoryRequest(query: string, filters: Omit<HistoryQueryPayload, "text"> = {}) {
  return {
    SearchHistory: {
      query,
      session_id: filters.session_id ?? null,
      agent_id: filters.agent_id ?? null,
      provider: filters.provider ?? null,
      model: filters.model ?? null,
      workflow_id: filters.workflow_id ?? null,
      machine_id: filters.machine_id ?? null,
      repo_root: filters.repo_root ?? null,
      worktree_path: filters.worktree_path ?? null,
      kind: filters.kind ?? null,
      after_sequence: filters.after_sequence ?? null,
      limit: filters.limit ?? null,
    },
  }
}

export function semanticSearchHistoryRequest(
  query: string,
  filters: Omit<HistoryQueryPayload, "text" | "after_sequence" | "before_sequence"> & { mode?: SemanticSearchHistoryMode | null; cursor?: string | null } = {},
) {
  return {
    SemanticSearchHistory: {
      query,
      mode: filters.mode ?? null,
      session_id: filters.session_id ?? null,
      agent_id: filters.agent_id ?? null,
      provider: filters.provider ?? null,
      model: filters.model ?? null,
      workflow_id: filters.workflow_id ?? null,
      machine_id: filters.machine_id ?? null,
      repo_root: filters.repo_root ?? null,
      worktree_path: filters.worktree_path ?? null,
      kind: filters.kind ?? null,
      cursor: filters.cursor ?? null,
      limit: filters.limit ?? null,
    },
  }
}

export function launchProviderRunRequest(
  sessionId: string,
  provider: string,
  accountProfile: string,
  model: string,
  effort: string,
  agentId?: string | null,
  native?: {
    structuredEndpoint?: string | null
    providerSessionId?: string | null
    nativeTui?: boolean | null
  } | null,
) {
  const normalizedModel = provider === "codex" && model.startsWith("codex/")
    ? model.slice("codex/".length)
    : model
  return {
    LaunchProviderRun: {
      session_id: sessionId,
      agent_id: agentId ?? null,
      adapter_key: provider,
      provider,
      account_profile: accountProfile,
      model: normalizedModel,
      variant: effort.trim() || null,
      structured_endpoint: native?.structuredEndpoint ?? null,
      provider_session_id: native?.providerSessionId ?? null,
      native_tui: native?.nativeTui ?? false,
    },
  }
}

export function captureScreenshotRequest(sessionId: string, attachmentId: string) {
  return {
    CaptureScreenshot: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function storeTransferredFileRequest(sessionId: string, attachmentId: string, sourcePath: string, displayName?: string) {
  return {
    StoreTransferredFile: {
      session_id: sessionId,
      attachment_id: attachmentId,
      source_path: sourcePath,
      display_name: displayName ?? null,
    },
  }
}

export function resizeTerminalRequest(sessionId: string, cols: number, rows: number) {
  return {
    ResizeTerminal: {
      session_id: sessionId,
      cols,
      rows,
    },
  }
}

export function sendTerminalInputRequest(
  sessionId: string,
  attachmentId: string,
  input: string | Uint8Array,
  providerRunId?: string | null,
) {
  const bytes = typeof input === "string" ? Buffer.from(input, "utf8") : Buffer.from(input)
  return {
    SendTerminalInput: {
      session_id: sessionId,
      attachment_id: attachmentId,
      provider_run_id: providerRunId ?? null,
      data_base64: bytes.toString("base64"),
    },
  }
}

export function pumpTerminalOutputRequest(sessionId: string, attachmentId: string) {
  return {
    PumpTerminalOutput: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function appendNativeProviderOutputRequest(
  sessionId: string,
  attachmentId: string,
  providerRunId: string,
  kind: "provider_output" | "provider_reasoning" | "provider_tool" | "provider_error" | "provider_status",
  text: string,
  mergeKey?: string | null,
) {
  return {
    AppendNativeProviderOutput: {
      session_id: sessionId,
      attachment_id: attachmentId,
      provider_run_id: providerRunId,
      kind,
      merge_key: mergeKey ?? null,
      text,
    },
  }
}

export function submitPromptRequest(
  sessionId: string,
  attachmentId: string,
  targetAgentId: string | null,
  prompt: string,
  attachments: PromptAttachmentPart[],
) {
  return {
    SubmitPrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
      target_agent_id: targetAgentId,
      prompt,
      attachments,
    },
  }
}

export function completePromptRequest(sessionId: string) {
  return {
    CompletePrompt: {
      session_id: sessionId,
    },
  }
}

export function cancelActivePromptRequest(sessionId: string, attachmentId: string) {
  return {
    CancelActivePrompt: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function pollRuntimeNoticesRequest(sessionId: string, attachmentId: string) {
  return {
    PollRuntimeNotices: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function respondToInteractionRequest(
  sessionId: string,
  interactionId: string,
  choiceId: string,
  customReply?: string | null,
) {
  return {
    RespondToInteraction: {
      session_id: sessionId,
      interaction_id: interactionId,
      choice_id: choiceId,
      custom_reply: customReply ?? null,
    },
  }
}

export function requestNativeProviderInteractionRequest(
  sessionId: string,
  agentId: string,
  interactionId: string,
  title: string | null,
  message: string,
  timeoutSec = 300,
) {
  return {
    RequestNativeProviderInteraction: {
      session_id: sessionId,
      agent_id: agentId,
      interaction_id: interactionId,
      level: "warning",
      title,
      message,
      choices: [
        {
          id: "allow_once",
          label: "Allow once",
          reply: "allow",
          style: "primary",
        },
        {
          id: "deny",
          label: "Deny",
          reply: "deny",
          style: "danger",
        },
      ],
      custom_choice: null,
      timeout_sec: timeoutSec,
      default_on_timeout: "deny",
    },
  }
}

export function spawnAgentRequest(
  sessionId: string,
  provider?: string | null,
  alias?: string,
  model?: string | null,
  worktreeId?: string,
  effort?: string | null,
  executionMode?: "build" | "plan",
  permissionLevel?: "required" | "yolo",
  kernelRef?: string,
  worktreePlacement?: Record<string, unknown>,
  sliceRef?: string,
) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider: provider ?? null,
      alias: alias ?? null,
      model: model ?? null,
      effort: effort ?? null,
      execution_mode: executionMode ?? null,
      permission_level: permissionLevel ?? null,
      worktree_id: worktreeId ?? null,
      kernel_ref: kernelRef ?? null,
      slice_ref: sliceRef ?? null,
      worktree_placement: worktreePlacement ?? null,
    },
  }
}

export function listSlicesRequest() {
  return { ListSlices: null }
}

export function createSliceRequest(options: {
  name: string
  backend?: "local_docker" | "ssh_docker"
  os?: string
  workspaceMount?: string | null
  workerKernelRef?: string | null
  displayUrl?: string | null
}) {
  return {
    CreateSlice: {
      name: options.name,
      backend: options.backend ?? "local_docker",
      os: options.os ?? "linux",
      workspace_mount: options.workspaceMount ?? null,
      worker_kernel_ref: options.workerKernelRef ?? null,
      display_url: options.displayUrl ?? null,
    },
  }
}

export function getSliceRequest(sliceRef: string) {
  return { GetSlice: { slice_ref: sliceRef } }
}

export function startSliceRequest(sliceRef: string) {
  return { StartSlice: { slice_ref: sliceRef } }
}

export function stopSliceRequest(sliceRef: string) {
  return { StopSlice: { slice_ref: sliceRef } }
}

export function deleteSliceRequest(sliceRef: string) {
  return { DeleteSlice: { slice_ref: sliceRef } }
}

export function importSliceProviderAuthRequest(sliceRef: string, provider: string) {
  return {
    ImportSliceProviderAuth: {
      slice_ref: sliceRef,
      provider,
    },
  }
}

export function getSliceDisplayEndpointRequest(sliceRef: string) {
  return { GetSliceDisplayEndpoint: { slice_ref: sliceRef } }
}

export function updateAgentConfigRequest(options: {
  sessionId: string
  agentId: string
  executionMode?: "build" | "plan" | null
  clearExecutionMode?: boolean
  permissionLevel?: "required" | "yolo" | null
  clearPermissionLevel?: boolean
  workspaceId?: string | null
  clearWorkspaceId?: boolean
  worktreeId?: string | null
  clearWorktreeId?: boolean
}) {
  return {
    UpdateAgentConfig: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      execution_mode: options.executionMode ?? null,
      clear_execution_mode: options.clearExecutionMode ?? false,
      permission_level: options.permissionLevel ?? null,
      clear_permission_level: options.clearPermissionLevel ?? false,
      workspace_id: options.workspaceId ?? null,
      clear_workspace_id: options.clearWorkspaceId ?? false,
      worktree_id: options.worktreeId ?? null,
      clear_worktree_id: options.clearWorktreeId ?? false,
    },
  }
}

export function updateAgentProfileRequest(options: {
  sessionId: string
  agentId: string
  provider?: string | null
  model?: string | null
  effort?: string | null
  clearEffort?: boolean
}) {
  return {
    UpdateAgentProfile: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      provider: options.provider ?? null,
      model: options.model ?? null,
      effort: options.effort ?? null,
      clear_effort: options.clearEffort ?? false,
    },
  }
}

export type AgentSubstituteAction =
  | { Add: { provider: string; model: string; variant?: string | null; kernel_id?: string | null; worktree_id?: string | null } }
  | { Remove: { index: number } }
  | { Clear: Record<string, never> }
  | { SetTimeout: { timeout_ms?: number | null } }
  | { Activate: { index: number; reason?: string | null } }
  | { Primary: Record<string, never> }

export function updateAgentSubstitutesRequest(options: {
  sessionId: string
  agentId: string
  action: AgentSubstituteAction
}) {
  return {
    UpdateAgentSubstitutes: {
      session_id: options.sessionId,
      agent_id: options.agentId,
      action: options.action,
    },
  }
}

export function moveAgentToRemoteRequest(sessionId: string, agentRef: string, machineRef: string) {
  return {
    MoveAgentToRemote: {
      session_id: sessionId,
      agent_ref: agentRef,
      machine_ref: machineRef,
    },
  }
}

export function destroyAgentRequest(sessionId: string, agentId: string) {
  return {
    DestroyAgent: {
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function focusAgentRequest(sessionId: string, agentId: string) {
  return {
    FocusAgent: {
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function cycleAgentFocusRequest(sessionId: string) {
  return {
    CycleAgentFocus: {
      session_id: sessionId,
    },
  }
}

export function listAgentsRequest(sessionId: string) {
  return {
    ListAgents: {
      session_id: sessionId,
    },
  }
}

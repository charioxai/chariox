import type {
  PromptAttachmentPart,
} from "./kernel-types.js"

export * from "./ipc-workflow-requests.js"
export * from "./ipc-workspace-requests.js"
export * from "./ipc-remote-connection-requests.js"
export * from "./ipc-relay-control-requests.js"
export * from "./ipc-extension-requests.js"
export * from "./ipc-history-requests.js"
export * from "./ipc-session-requests.js"

export function deleteKernelRequest() {
  return { DeleteKernel: null }
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

export function getDaemonHealthRequest() {
  return { GetDaemonHealth: null }
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

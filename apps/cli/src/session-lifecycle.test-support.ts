import type { CliOptions, RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import type {
  AttachedCliTransitionState,
  DetachedCliTransitionState,
} from "./session-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

const cliOptions: CliOptions = {
  clientId: "cli-1",
  provider: "opencode",
  model: "openai/gpt-5",
  accountProfile: "default",
  effort: "medium",
  workspace: "/tmp/workspace",
  worktree: "/tmp/workspace",
}

const waitingRoomState: WaitingRoomState = {
  focus: "new",
  sessionIndex: 0,
  machineIndex: 0,
  remoteKernelIndex: 0,
  terminalIndex: 0,
  worktreeSelectionId: "existing:/tmp/workspace",
  workspaceLiveSyncMode: "off",
  providerId: "opencode",
  modelId: "openai/gpt-5",
  effort: "medium",
  themeId: "opencode",
  introStep: 0,
  keyState: { up: false, down: false, left: false, right: false },
}

const detachedState: DetachedCliTransitionState = {
  centerMode: "transcript",
  createdSession: false,
  session: {
    id: "no-session",
    alias: null,
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 0,
    status: "Parked",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    config_state: { version: 0, values: {} },
  },
  providerActivityLabel: null,
  activeStatusLabel: null,
  agentPaneEntries: {},
  agentPanePreviews: {},
  agentActivityLabels: {},
  streamingAgentId: null,
  submitting: false,
  working: false,
  fatalError: null,
  daemonDisconnected: false,
  nextHistoryCursor: null,
  statusLine: "No session attached.",
  waitingRoomState,
}

const attachedState = (session: RuntimeSession): AttachedCliTransitionState => ({
  centerMode: "transcript",
  createdSession: false,
  session,
  providerActivityLabel: null,
  activeStatusLabel: null,
  fatalError: null,
  daemonDisconnected: false,
  streamingAgentId: "agent-a",
  submitting: false,
  working: false,
  statusLine: "Connected",
})

export function createBaseDeps(overrides: Record<string, unknown> = {}) {
  const calls: string[] = []
  const currentAttachment: RuntimeAttachment | null = {
    id: "att-1",
    session_id: "session-1",
  }

  const deps = {
    cliOptions: { ...cliOptions },
    connectedStatus: "Connected",
    waitingRoomState: () => waitingRoomState,
    attachmentState: () => currentAttachment,
    deriveDetachedCliTransitionState: () => detachedState,
    deriveAttachedCliTransitionState: ({ session }: { session: RuntimeSession }) => attachedState(session),
    clearPendingPromptAttachments: () => calls.push("clearPendingPromptAttachments"),
    clearActiveToolLabels: () => calls.push("clearActiveToolLabels"),
    clearWorkflows: () => calls.push("clearWorkflows"),
    clearAgentPaneRuntime: () => calls.push("clearAgentPaneRuntime"),
    clearDirectoryTree: () => calls.push("clearDirectoryTree"),
    clearTranscript: () => calls.push("clearTranscript"),
    refreshResponseLayout: () => calls.push("refreshResponseLayout"),
    resetWorkspaceScreen: () => calls.push("resetWorkspaceScreen"),
    resetStopRequestInFlight: () => calls.push("resetStopRequestInFlight"),
    bumpHistoryLoadGeneration: () => calls.push("bumpHistoryLoadGeneration"),
    reconcileWaitingRoom: () => calls.push("reconcileWaitingRoom"),
    refreshWaitingRoomData: async () => { calls.push("refreshWaitingRoomData") },
    requestRender: () => calls.push("requestRender"),
    clearPromptInput: () => calls.push("clearPromptInput"),
    syncPromptTextSnapshot: () => calls.push("syncPromptTextSnapshot"),
    blurPromptInput: () => calls.push("blurPromptInput"),
    focusPromptInput: () => calls.push("focusPromptInput"),
    layoutPreference: () => null,
    setMultiAgentResponseLayout: () => calls.push("setMultiAgentResponseLayout"),
    setAttachmentState: () => calls.push("setAttachmentState"),
    setProviderRunState: () => calls.push("setProviderRunState"),
    setCenterMode: () => calls.push("setCenterMode"),
    setCreatedSessionState: () => calls.push("setCreatedSessionState"),
    setSessionState: () => calls.push("setSessionState"),
    setProviderActivityLabel: () => calls.push("setProviderActivityLabel"),
    setActiveStatusLabel: () => calls.push("setActiveStatusLabel"),
    setAgentPaneEntries: () => calls.push("setAgentPaneEntries"),
    setAgentPanePreviews: () => calls.push("setAgentPanePreviews"),
    setAgentActivityLabels: () => calls.push("setAgentActivityLabels"),
    setStreamingAgentId: () => calls.push("setStreamingAgentId"),
    setSubmitting: () => calls.push("setSubmitting"),
    setWorking: () => calls.push("setWorking"),
    setFatalError: () => calls.push("setFatalError"),
    setDaemonDisconnected: () => calls.push("setDaemonDisconnected"),
    setNextHistoryCursor: () => calls.push("setNextHistoryCursor"),
    setSessionHydratingState: () => calls.push("setSessionHydratingState"),
    setHistoryLoadingState: () => calls.push("setHistoryLoadingState"),
    setStatusLine: () => calls.push("setStatusLine"),
    updateSessionChrome: () => calls.push("updateSessionChrome"),
    refreshSplitPaneFocusRepaint: () => calls.push("refreshSplitPaneFocusRepaint"),
    attachToSession: async () => {
      calls.push("attachToSession")
      throw new Error("should not be called")
    },
    getSessionState: async () => {
      calls.push("getSessionState")
      throw new Error("should not be called")
    },
    launchProviderRun: async () => {
      calls.push("launchProviderRun")
      throw new Error("should not be called")
    },
    tryGetProviderRun: async () => {
      calls.push("tryGetProviderRun")
      return null
    },
    setProviderCatalogState: () => calls.push("setProviderCatalogState"),
    setTerminalCommandCatalogState: () => calls.push("setTerminalCommandCatalogState"),
    syncCliProviderSelection: () => calls.push("syncCliProviderSelection"),
    getProviderCatalog: async () => {
      calls.push("getProviderCatalog")
      return {}
    },
    getTerminalCommandCatalog: async () => {
      calls.push("getTerminalCommandCatalog")
      return {}
    },
    primeAttachedSessionBinding: async () => {
      calls.push("primeAttachedSessionBinding")
    },
    hydrateAttachedSessionBinding: async () => {
      calls.push("hydrateAttachedSessionBinding")
      return detachedState.session
    },
    setAvailableSessions: () => calls.push("setAvailableSessions"),
    listSessions: async () => {
      calls.push("listSessions")
      return []
    },
    scheduleShortViewportHistoryCheck: () => calls.push("scheduleShortViewportHistoryCheck"),
    detachAttachment: async () => { calls.push("detachAttachment") },
    syncKernelEventSubscription: async () => { calls.push("syncKernelEventSubscription") },
    logAttachedProviderRun: () => calls.push("logAttachedProviderRun"),
    ...overrides,
  }

  return { deps, calls }
}

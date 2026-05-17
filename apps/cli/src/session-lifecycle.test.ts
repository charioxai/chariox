import assert from "node:assert/strict"
import test from "node:test"

import type { CliOptions, RuntimeAttachment, RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import type {
  AttachedCliTransitionState,
  DetachedCliTransitionState,
} from "./session-state.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type { ProviderPreferences } from "./preferences.js"

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
  submitting: false,
  working: false,
  statusLine: "Connected",
})

function createBaseDeps(overrides: Record<string, unknown> = {}) {
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
    syncCliProviderSelection: () => calls.push("syncCliProviderSelection"),
    getProviderCatalog: async () => {
      calls.push("getProviderCatalog")
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

test("attachBinding is a no-op when already attached to the target session", async () => {
  const { deps, calls } = createBaseDeps()
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-1" }, false)

  assert.deepEqual(calls, [])
})

test("transitionToNoSession resets session-bound state and refreshes the waiting room", async () => {
  const { deps, calls } = createBaseDeps()
  const controller = createSessionLifecycleController(deps as never)

  await controller.transitionToNoSession("Session deleted.")

  assert.deepEqual(calls, [
    "setAttachmentState",
    "setProviderRunState",
    "clearPendingPromptAttachments",
    "resetWorkspaceScreen",
    "clearWorkflows",
    "setCenterMode",
    "clearDirectoryTree",
    "clearActiveToolLabels",
    "setProviderActivityLabel",
    "setActiveStatusLabel",
    "setCreatedSessionState",
    "setSessionState",
    "refreshResponseLayout",
    "bumpHistoryLoadGeneration",
    "clearTranscript",
    "setAgentPaneEntries",
    "setAgentPanePreviews",
    "setAgentActivityLabels",
    "setStreamingAgentId",
    "clearAgentPaneRuntime",
    "setSubmitting",
    "setWorking",
    "resetStopRequestInFlight",
    "setFatalError",
    "setDaemonDisconnected",
    "setNextHistoryCursor",
    "setSessionHydratingState",
    "setHistoryLoadingState",
    "setStatusLine",
    "updateSessionChrome",
    "clearPromptInput",
    "syncPromptTextSnapshot",
    "blurPromptInput",
    "reconcileWaitingRoom",
    "refreshWaitingRoomData",
    "requestRender",
  ])
})

test("attachBinding reattaches and hydrates the attached session before restoring the attached state", async () => {
  const events: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    alias: "feature",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-2",
    attachment_ids: ["att-2"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 6,
    agents: [],
    config_state: { version: 1, values: {} },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    clearPendingPromptAttachments: () => events.push("clearPendingPromptAttachments"),
    clearWorkflows: () => events.push("clearWorkflows"),
    bumpHistoryLoadGeneration: () => events.push("bumpHistoryLoadGeneration"),
    resetWorkspaceScreen: () => events.push("resetWorkspaceScreen"),
    attachToSession: async () => {
      events.push("attachToSession")
      return { id: "att-2", session_id: "session-2" }
    },
    getSessionState: async () => {
      events.push("getSessionState")
      return attachedSession
    },
    tryGetProviderRun: async () => {
      events.push("tryGetProviderRun")
      return {
        id: "run-2",
        session_id: "session-2",
        agent_instance_id: "agent-a",
        adapter_key: "opencode",
        provider: "opencode",
        account_profile: "default",
        model: "gpt-5",
        variant: "medium",
        usage_tokens_total: null,
        state: "Running",
      }
    },
    setProviderRunState: () => events.push("setProviderRunState"),
    syncCliProviderSelection: () => events.push("syncCliProviderSelection"),
    setProviderCatalogState: () => events.push("setProviderCatalogState"),
    getProviderCatalog: async () => {
      events.push("getProviderCatalog")
      return {}
    },
    primeAttachedSessionBinding: async () => {
      events.push("primeAttachedSessionBinding")
    },
    reconcileWaitingRoom: () => events.push("reconcileWaitingRoom"),
    hydrateAttachedSessionBinding: async () => {
      events.push("hydrateAttachedSessionBinding")
      return attachedSession
    },
    setAttachmentState: () => events.push("setAttachmentState"),
    setCreatedSessionState: () => events.push("setCreatedSessionState"),
    setSessionState: () => events.push("setSessionState"),
    setCenterMode: () => events.push("setCenterMode"),
    clearDirectoryTree: () => events.push("clearDirectoryTree"),
    clearActiveToolLabels: () => events.push("clearActiveToolLabels"),
    setProviderActivityLabel: () => events.push("setProviderActivityLabel"),
    setActiveStatusLabel: () => events.push("setActiveStatusLabel"),
    setFatalError: () => events.push("setFatalError"),
    setDaemonDisconnected: () => events.push("setDaemonDisconnected"),
    setSubmitting: () => events.push("setSubmitting"),
    setWorking: () => events.push("setWorking"),
    setStatusLine: () => events.push("setStatusLine"),
    setSessionHydratingState: () => events.push("setSessionHydratingState"),
    updateSessionChrome: () => events.push("updateSessionChrome"),
    focusPromptInput: () => events.push("focusPromptInput"),
    setMultiAgentResponseLayout: () => events.push("setMultiAgentResponseLayout"),
    syncKernelEventSubscription: async () => { events.push("syncKernelEventSubscription") },
    setAvailableSessions: () => events.push("setAvailableSessions"),
    listSessions: async () => {
      events.push("listSessions")
      return [attachedSession]
    },
    scheduleShortViewportHistoryCheck: () => events.push("scheduleShortViewportHistoryCheck"),
    logAttachedProviderRun: () => events.push("logAttachedProviderRun"),
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-2" }, false)

  assert.deepEqual(events, [
    "clearPendingPromptAttachments",
    "bumpHistoryLoadGeneration",
    "setSessionHydratingState",
    "attachToSession",
    "getSessionState",
    "setMultiAgentResponseLayout",
    "setCreatedSessionState",
    "setSessionState",
    "setCenterMode",
    "setAttachmentState",
    "clearDirectoryTree",
    "resetWorkspaceScreen",
    "clearWorkflows",
    "clearActiveToolLabels",
    "setProviderActivityLabel",
    "setActiveStatusLabel",
    "setFatalError",
    "setDaemonDisconnected",
    "setSubmitting",
    "setWorking",
    "setStatusLine",
    "updateSessionChrome",
    "focusPromptInput",
    "primeAttachedSessionBinding",
    "setSessionHydratingState",
    "syncKernelEventSubscription",
    "tryGetProviderRun",
    "logAttachedProviderRun",
    "setProviderRunState",
    "syncCliProviderSelection",
    "getProviderCatalog",
    "setProviderCatalogState",
    "reconcileWaitingRoom",
    "hydrateAttachedSessionBinding",
    "setMultiAgentResponseLayout",
    "setCreatedSessionState",
    "setSessionState",
    "setCenterMode",
    "setAttachmentState",
    "clearDirectoryTree",
    "resetWorkspaceScreen",
    "clearWorkflows",
    "clearActiveToolLabels",
    "setProviderActivityLabel",
    "setActiveStatusLabel",
    "setFatalError",
    "setDaemonDisconnected",
    "setSubmitting",
    "setWorking",
    "setStatusLine",
    "updateSessionChrome",
    "focusPromptInput",
    "listSessions",
    "setAvailableSessions",
    "scheduleShortViewportHistoryCheck",
  ])
})

test("attachBinding launches a provider run with provider and effort in the correct positions", async () => {
  const launched: Array<{
    sessionId: string
    provider: string
    accountProfile: string
    model: string
    effort: string
    targetAgentId: string | null | undefined
  }> = []
  const attachedSession: RuntimeSession = {
    id: "session-3",
    alias: "feature",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-3"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-focus",
    max_agents: 6,
    agents: [{
      id: "agent-focus",
      agent_ref: "agent-focus",
      session_id: "session-3",
      alias: null,
      provider: "codex",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      worktree_id: "/tmp/workspace",
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-3", session_id: "session-3" }),
    getSessionState: async () => attachedSession,
    launchProviderRun: async (
      sessionId: string,
      provider: string,
      accountProfile: string,
      model: string,
      effort: string,
      targetAgentId?: string | null,
    ) => {
      launched.push({ sessionId, provider, accountProfile, model, effort, targetAgentId })
      return {
        id: "run-3",
        session_id: sessionId,
        agent_instance_id: targetAgentId ?? null,
        adapter_key: provider,
        provider,
        account_profile: accountProfile,
        model,
        variant: effort,
        usage_tokens_total: null,
        state: "Running",
      }
    },
    setProviderRunState: () => {},
    setProviderCatalogState: () => {},
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    setAttachmentState: () => {},
    setCreatedSessionState: () => {},
    setSessionState: () => {},
    setCenterMode: () => {},
    clearDirectoryTree: () => {},
    resetWorkspaceScreen: () => {},
    clearWorkflows: () => {},
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    setFatalError: () => {},
    setDaemonDisconnected: () => {},
    setSubmitting: () => {},
    setWorking: () => {},
    setStatusLine: () => {},
    updateSessionChrome: () => {},
    focusPromptInput: () => {},
    setMultiAgentResponseLayout: () => {},
    syncKernelEventSubscription: async () => {},
    setAvailableSessions: () => {},
    listSessions: async () => [],
    scheduleShortViewportHistoryCheck: () => {},
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding(
    { id: "session-3" },
    true,
    { provider: "codex", model: "codex/gpt-5.4-mini", effort: "low" },
  )

  assert.deepEqual(launched, [
    {
      sessionId: "session-3",
      provider: "codex",
      accountProfile: "default",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      targetAgentId: "agent-focus",
    },
  ])
})

test("attachBinding skips provider launch when existing session exposes no visible agents", async () => {
  const attachedSession: RuntimeSession = {
    id: "session-hidden-focus",
    alias: "shared",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-hidden-focus"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    config_state: { version: 1, values: {} },
  }
  let launchCalled = false
  const providerRuns: Array<RuntimeProviderRun | null> = []
  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-hidden-focus", session_id: "session-hidden-focus" }),
    getSessionState: async () => attachedSession,
    launchProviderRun: async () => {
      launchCalled = true
      throw new Error("should not launch a provider run")
    },
    setProviderRunState: (run: RuntimeProviderRun | null) => { providerRuns.push(run) },
    setProviderCatalogState: () => {},
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    setAttachmentState: () => {},
    setCreatedSessionState: () => {},
    setSessionState: () => {},
    setCenterMode: () => {},
    clearDirectoryTree: () => {},
    resetWorkspaceScreen: () => {},
    clearWorkflows: () => {},
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    setFatalError: () => {},
    setDaemonDisconnected: () => {},
    setSubmitting: () => {},
    setWorking: () => {},
    setStatusLine: () => {},
    updateSessionChrome: () => {},
    focusPromptInput: () => {},
    setMultiAgentResponseLayout: () => {},
    syncKernelEventSubscription: async () => {},
    setAvailableSessions: () => {},
    listSessions: async () => [],
    scheduleShortViewportHistoryCheck: () => {},
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-hidden-focus" }, false)

  assert.equal(launchCalled, false)
  assert.deepEqual(providerRuns, [null])
})

test("attachBinding restores the focused agent runtime profile for existing sessions", async () => {
  const launched: Array<{
    provider: string
    model: string
    effort: string
  }> = []
  const attachedSession: RuntimeSession = {
    id: "session-4",
    alias: "parked",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Parked",
    active_provider_run_id: null,
    attachment_ids: ["att-4"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-focus",
    max_agents: 6,
    agents: [{
      id: "agent-focus",
      agent_ref: "agent-focus",
      session_id: "session-4",
      alias: null,
      provider: "codex",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      worktree_id: null,
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-4", session_id: "session-4" }),
    getSessionState: async () => attachedSession,
    launchProviderRun: async (
      _sessionId: string,
      provider: string,
      _accountProfile: string,
      model: string,
      effort: string,
    ) => {
      launched.push({ provider, model, effort })
      return {
        id: "run-4",
        session_id: "session-4",
        agent_instance_id: "agent-focus",
        adapter_key: provider,
        provider,
        account_profile: "default",
        model,
        variant: effort,
        usage_tokens_total: null,
        state: "Running",
      }
    },
    setProviderRunState: () => {},
    setProviderCatalogState: () => {},
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    setAttachmentState: () => {},
    setCreatedSessionState: () => {},
    setSessionState: () => {},
    setCenterMode: () => {},
    clearDirectoryTree: () => {},
    resetWorkspaceScreen: () => {},
    clearWorkflows: () => {},
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    setFatalError: () => {},
    setDaemonDisconnected: () => {},
    setSubmitting: () => {},
    setWorking: () => {},
    setStatusLine: () => {},
    updateSessionChrome: () => {},
    focusPromptInput: () => {},
    setMultiAgentResponseLayout: () => {},
    syncKernelEventSubscription: async () => {},
    setAvailableSessions: () => {},
    listSessions: async () => [],
    scheduleShortViewportHistoryCheck: () => {},
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding(
    { id: "session-4" },
    false,
    { provider: "opencode", model: "openai/gpt-5.4", effort: "high" },
  )

  assert.deepEqual(launched, [{
    provider: "codex",
    model: "codex/gpt-5.4-mini",
    effort: "low",
  }])
})

test("attachBinding syncs CLI provider selection from an existing active provider run", async () => {
  const syncedSelections: Array<{
    provider: string
    model: string
    effort: string
  }> = []
  const attachedSession: RuntimeSession = {
    id: "session-4b",
    alias: "parked",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-4b",
    attachment_ids: ["att-4b"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-focus",
    max_agents: 6,
    agents: [{
      id: "agent-focus",
      agent_ref: "agent-focus",
      session_id: "session-4b",
      alias: null,
      provider: "opencode",
      model: "openai/gpt-5.4",
      effort: "high",
      worktree_id: null,
      state: "Idle",
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-4b", session_id: "session-4b" }),
    getSessionState: async () => attachedSession,
    tryGetProviderRun: async () => ({
      id: "run-4b",
      session_id: "session-4b",
      agent_instance_id: "agent-focus",
      adapter_key: "codex",
      provider: "codex",
      account_profile: "default",
      model: "codex/gpt-5.4-mini",
      variant: "low",
      usage_tokens_total: null,
      state: "Running",
    }),
    setProviderRunState: () => {},
    syncCliProviderSelection: (selection: Required<ProviderPreferences> & { provider: string }) => {
      syncedSelections.push(selection)
    },
    setProviderCatalogState: () => {},
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    setAttachmentState: () => {},
    setCreatedSessionState: () => {},
    setSessionState: () => {},
    setCenterMode: () => {},
    clearDirectoryTree: () => {},
    resetWorkspaceScreen: () => {},
    clearWorkflows: () => {},
    clearActiveToolLabels: () => {},
    setProviderActivityLabel: () => {},
    setActiveStatusLabel: () => {},
    setFatalError: () => {},
    setDaemonDisconnected: () => {},
    setSubmitting: () => {},
    setWorking: () => {},
    setStatusLine: () => {},
    updateSessionChrome: () => {},
    focusPromptInput: () => {},
    setMultiAgentResponseLayout: () => {},
    syncKernelEventSubscription: async () => {},
    setAvailableSessions: () => {},
    listSessions: async () => [],
    scheduleShortViewportHistoryCheck: () => {},
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding(
    { id: "session-4b" },
    false,
    { provider: "opencode", model: "openai/gpt-5.4", effort: "high" },
  )

  assert.deepEqual(syncedSelections, [{
    provider: "codex",
    model: "codex/gpt-5.4-mini",
    effort: "low",
  }])
})

test("attachBinding keeps the CLI attached when post-attach refresh steps fail", async () => {
  const events: string[] = []
  const warnings: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    alias: "feature",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-2",
    attachment_ids: ["att-2"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 6,
    agents: [],
    config_state: { version: 1, values: {} },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-2", session_id: "session-2" }),
    getSessionState: async () => attachedSession,
    tryGetProviderRun: async () => ({
      id: "run-2",
      session_id: "session-2",
      agent_instance_id: "agent-a",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "Running",
    }),
    setAttachmentState: () => events.push("setAttachmentState"),
    resetWorkspaceScreen: () => events.push("resetWorkspaceScreen"),
    setCreatedSessionState: () => events.push("setCreatedSessionState"),
    setSessionState: () => events.push("setSessionState"),
    setCenterMode: () => events.push("setCenterMode"),
    clearDirectoryTree: () => events.push("clearDirectoryTree"),
    clearActiveToolLabels: () => events.push("clearActiveToolLabels"),
    setProviderActivityLabel: () => events.push("setProviderActivityLabel"),
    setActiveStatusLabel: () => events.push("setActiveStatusLabel"),
    setFatalError: () => events.push("setFatalError"),
    setDaemonDisconnected: () => events.push("setDaemonDisconnected"),
    setSubmitting: () => events.push("setSubmitting"),
    setWorking: () => events.push("setWorking"),
    setStatusLine: () => events.push("setStatusLine"),
    updateSessionChrome: () => events.push("updateSessionChrome"),
    focusPromptInput: () => events.push("focusPromptInput"),
    syncKernelEventSubscription: async () => { throw new Error("subscribe failed") },
    getProviderCatalog: async () => { throw new Error("catalog down") },
    hydrateAttachedSessionBinding: async () => { throw new Error("hydrate failed") },
    listSessions: async () => { throw new Error("list failed") },
    logWarning: (message: string) => warnings.push(message),
  })

  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-2" }, false)

  assert.equal(events.includes("setAttachmentState"), true)
  assert.equal(events.includes("setSessionState"), true)
  assert.equal(events.includes("updateSessionChrome"), true)
  assert.equal(events.includes("focusPromptInput"), true)
  assert.deepEqual(warnings, [
    "failed to synchronize kernel event subscription after attach",
    "failed to refresh provider catalog after attach",
    "failed to hydrate attached session after attach",
    "failed to refresh session list after attach",
  ])
})

test("attachBinding synchronizes kernel event subscription immediately after applying attached state", async () => {
  const events: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    alias: "feature",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-2",
    attachment_ids: ["att-2"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 6,
    agents: [],
    config_state: { version: 1, values: {} },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-2", session_id: "session-2" }),
    getSessionState: async () => attachedSession,
    tryGetProviderRun: async () => ({
      id: "run-2",
      session_id: "session-2",
      agent_instance_id: "agent-a",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "Running",
    }),
    setCreatedSessionState: () => events.push("setCreatedSessionState"),
    setSessionState: () => events.push("setSessionState"),
    setCenterMode: () => events.push("setCenterMode"),
    setAttachmentState: () => events.push("setAttachmentState"),
    resetWorkspaceScreen: () => events.push("resetWorkspaceScreen"),
    clearDirectoryTree: () => events.push("clearDirectoryTree"),
    clearActiveToolLabels: () => events.push("clearActiveToolLabels"),
    setProviderActivityLabel: () => events.push("setProviderActivityLabel"),
    setActiveStatusLabel: () => events.push("setActiveStatusLabel"),
    setFatalError: () => events.push("setFatalError"),
    setDaemonDisconnected: () => events.push("setDaemonDisconnected"),
    setSubmitting: () => events.push("setSubmitting"),
    setWorking: () => events.push("setWorking"),
    setStatusLine: () => events.push("setStatusLine"),
    updateSessionChrome: () => events.push("updateSessionChrome"),
    focusPromptInput: () => events.push("focusPromptInput"),
    setMultiAgentResponseLayout: () => events.push("setMultiAgentResponseLayout"),
    syncKernelEventSubscription: async () => { events.push("syncKernelEventSubscription") },
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    listSessions: async () => [],
  })

  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-2" }, false)

  assert.deepEqual(events.slice(0, 15), [
    "setMultiAgentResponseLayout",
    "setCreatedSessionState",
    "setSessionState",
    "setCenterMode",
    "setAttachmentState",
    "clearDirectoryTree",
    "resetWorkspaceScreen",
    "clearActiveToolLabels",
    "setProviderActivityLabel",
    "setActiveStatusLabel",
    "setFatalError",
    "setDaemonDisconnected",
    "setSubmitting",
    "setWorking",
    "setStatusLine",
  ])
  assert.equal(
    events.indexOf("syncKernelEventSubscription") > events.indexOf("focusPromptInput"),
    true,
  )
})

test("attachBinding adopts the attached session response layout immediately", async () => {
  const appliedLayouts: string[] = []
  const repaintCalls: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    alias: "feature",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-2",
    attachment_ids: ["att-2"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 6,
    agents: [],
    config_state: {
      version: 1,
      values: { "ui.multiAgentResponseLayout": "split" },
      updated_by_attachment_id: null,
    },
  }

  const { deps } = createBaseDeps({
    attachmentState: () => null,
    layoutPreference: () => "individual",
    attachToSession: async () => ({ id: "att-2", session_id: "session-2" }),
    getSessionState: async () => attachedSession,
    tryGetProviderRun: async () => null,
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    resetWorkspaceScreen: () => {},
    setMultiAgentResponseLayout: (layout: string) => {
      appliedLayouts.push(layout)
    },
    refreshSplitPaneFocusRepaint: () => {
      repaintCalls.push("refresh")
    },
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-2" }, false)

  assert.deepEqual(appliedLayouts, ["split", "split"])
  assert.deepEqual(repaintCalls, ["refresh", "refresh"])
})

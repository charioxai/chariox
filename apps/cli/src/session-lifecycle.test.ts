import assert from "node:assert/strict"
import test from "node:test"

import type { CliOptions, RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import type {
  AttachedCliTransitionState,
  DetachedCliTransitionState,
} from "./session-state.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import type { WaitingRoomState } from "./waiting-room.js"

const cliOptions: CliOptions = {
  clientId: "cli-1",
  model: "openai/gpt-5",
  accountProfile: "default",
  effort: "medium",
  workspace: "/tmp/workspace",
  worktree: "/tmp/workspace",
}

const waitingRoomState: WaitingRoomState = {
  focus: "new",
  sessionIndex: 0,
  modelId: "openai/gpt-5",
  effort: "medium",
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
    setHistoryLoadingState: () => calls.push("setHistoryLoadingState"),
    setStatusLine: () => calls.push("setStatusLine"),
    updateSessionChrome: () => calls.push("updateSessionChrome"),
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
    getProviderCatalog: async () => {
      calls.push("getProviderCatalog")
      return {}
    },
    maybeResize: async () => { calls.push("maybeResize") },
    catchUpAttachedSession: async () => { calls.push("catchUpAttachedSession") },
    refreshAgentPanes: async () => { calls.push("refreshAgentPanes") },
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

test("attachBinding reattaches, catches up, and refreshes panes before restoring the attached state", async () => {
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
    setProviderCatalogState: () => events.push("setProviderCatalogState"),
    getProviderCatalog: async () => {
      events.push("getProviderCatalog")
      return {}
    },
    reconcileWaitingRoom: () => events.push("reconcileWaitingRoom"),
    maybeResize: async () => { events.push("maybeResize") },
    catchUpAttachedSession: async () => { events.push("catchUpAttachedSession") },
    refreshAgentPanes: async () => { events.push("refreshAgentPanes") },
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
    "attachToSession",
    "getSessionState",
    "tryGetProviderRun",
    "logAttachedProviderRun",
    "setProviderRunState",
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
    "syncKernelEventSubscription",
    "getProviderCatalog",
    "setProviderCatalogState",
    "reconcileWaitingRoom",
    "maybeResize",
    "catchUpAttachedSession",
    "getSessionState",
    "refreshAgentPanes",
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
    maybeResize: async () => { throw new Error("resize failed") },
    catchUpAttachedSession: async () => { throw new Error("catch-up failed") },
    refreshAgentPanes: async () => { throw new Error("pane refresh failed") },
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
    "failed to resize attached session",
    "failed to catch up attached session",
    "failed to refresh agent panes after attach",
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
    maybeResize: async () => {},
    catchUpAttachedSession: async () => {},
    refreshAgentPanes: async () => {},
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
    refreshAgentPanes: async () => {},
    resetWorkspaceScreen: () => {},
    setMultiAgentResponseLayout: (layout: string) => {
      appliedLayouts.push(layout)
    },
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-2" }, false)

  assert.deepEqual(appliedLayouts, ["split", "split"])
})

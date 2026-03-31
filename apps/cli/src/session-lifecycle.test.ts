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
    clearAgentPaneRuntime: () => calls.push("clearAgentPaneRuntime"),
    clearDirectoryTree: () => calls.push("clearDirectoryTree"),
    clearTranscript: () => calls.push("clearTranscript"),
    resetStopRequestInFlight: () => calls.push("resetStopRequestInFlight"),
    bumpHistoryLoadGeneration: () => calls.push("bumpHistoryLoadGeneration"),
    reconcileWaitingRoom: () => calls.push("reconcileWaitingRoom"),
    refreshWaitingRoomData: async () => { calls.push("refreshWaitingRoomData") },
    requestRender: () => calls.push("requestRender"),
    clearPromptInput: () => calls.push("clearPromptInput"),
    syncPromptTextSnapshot: () => calls.push("syncPromptTextSnapshot"),
    blurPromptInput: () => calls.push("blurPromptInput"),
    focusPromptInput: () => calls.push("focusPromptInput"),
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

import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createBaseDeps } from "./session-lifecycle.test-support.js"
import type { ProviderPreferences } from "./preferences.js"
import type { SessionListEntry } from "./sessions.js"

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
    setStreamingAgentId: () => events.push("setStreamingAgentId"),
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
    "setStreamingAgentId",
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
    "setStreamingAgentId",
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

test("attachBinding patches the waiting-room session row without listing every session", async () => {
  const appliedSessions: SessionListEntry[][] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    alias: "updated",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 10,
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
    tryGetProviderRun: async () => null,
    hydrateAttachedSessionBinding: async () => attachedSession,
    getAvailableSessions: () => [
      { id: "session-1", alias: "old", worktree_id: "/tmp/workspace", status: "Created" },
      {
        id: "session-2",
        alias: "stale",
        workspace_label: "Cached workspace",
        worktree_id: "/tmp/workspace",
        status: "Created",
      },
    ],
    setAvailableSessions: (sessions: SessionListEntry[]) => {
      appliedSessions.push(sessions)
    },
    listSessions: async () => {
      throw new Error("should not list sessions after attach when cached sessions are available")
    },
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-2" }, false)

  assert.deepEqual(appliedSessions.map((sessions) => sessions.map((session) => [session.id, session.alias ?? null, session.status])), [
    [
      ["session-1", "old", "Created"],
      ["session-2", "updated", "Active"],
    ],
  ])
  assert.equal(appliedSessions[0]?.[1]?.workspace_label, "Cached workspace")
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

test("attachBinding recovers when its launch target moves remote", async () => {
  const localSession: RuntimeSession = {
    id: "session-moving",
    alias: "moving",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-moving"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-moving",
    max_agents: 6,
    agents: [{
      id: "agent-moving",
      agent_ref: "agent-moving",
      session_id: "session-moving",
      alias: null,
      provider: "claude",
      model: "sonnet",
      effort: "medium",
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
  const remoteSession: RuntimeSession = {
    ...localSession,
    agents: [{
      ...localSession.agents[0]!,
      remote_execution: {
        worker_kernel_id: "worker-1",
        worker_machine_id: "machine-1",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }],
  }
  const appliedSessions: RuntimeSession[] = []
  const warnings: Array<{ message: string; fields: Record<string, unknown> | undefined }> = []
  let stateReads = 0
  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-moving", session_id: "session-moving" }),
    getSessionState: async () => {
      stateReads += 1
      return stateReads === 1 ? localSession : remoteSession
    },
    launchProviderRun: async () => {
      throw new Error("agent became remote-backed")
    },
    setSessionState: (session: RuntimeSession) => { appliedSessions.push(session) },
    setProviderRunState: () => {},
    setProviderCatalogState: () => {},
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    setAvailableSessions: () => {},
    listSessions: async () => [],
    logWarning: (message: string, fields?: Record<string, unknown>) => warnings.push({ message, fields }),
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-moving" }, false)

  assert.equal(stateReads, 2)
  assert.equal(appliedSessions.at(-1)?.agents[0]?.remote_execution?.worker_kernel_id, "worker-1")
  assert.deepEqual(warnings, [{
    message: "recovered attach-time provider launch after agent moved remote",
    fields: {
      session_id: "session-moving",
      agent_id: "agent-moving",
      worker_kernel_id: "worker-1",
    },
  }])
})

test("attachBinding does not mask unrelated provider launch failures", async () => {
  const attachedSession: RuntimeSession = {
    id: "session-launch-failure",
    alias: "failure",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-failure"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-local",
    max_agents: 6,
    agents: [{
      id: "agent-local",
      agent_ref: "agent-local",
      session_id: "session-launch-failure",
      alias: null,
      provider: "claude",
      model: "sonnet",
      effort: "medium",
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
  const launchError = new Error("provider authentication failed")
  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-failure", session_id: "session-launch-failure" }),
    getSessionState: async () => attachedSession,
    launchProviderRun: async () => { throw launchError },
  })
  const controller = createSessionLifecycleController(deps as never)

  await assert.rejects(
    controller.attachBinding({ id: "session-launch-failure" }, false),
    (error) => error === launchError,
  )
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

test("attachBinding skips provider launch when focused agent is stale", async () => {
  const warnings: Array<Record<string, unknown> | undefined> = []
  const providerRuns: Array<RuntimeProviderRun | null> = []
  const attachedSession: RuntimeSession = {
    id: "session-stale-focus",
    alias: "shared",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-stale-focus"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "missing-agent",
    max_agents: 6,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-stale-focus",
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
  let launchCalled = false
  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "att-stale-focus", session_id: "session-stale-focus" }),
    getSessionState: async () => attachedSession,
    launchProviderRun: async () => {
      launchCalled = true
      throw new Error("should not launch a provider run")
    },
    setProviderRunState: (run: RuntimeProviderRun | null) => { providerRuns.push(run) },
    setProviderCatalogState: () => {},
    getProviderCatalog: async () => ({}),
    hydrateAttachedSessionBinding: async (_sessionId: string, _attachmentId: string, session: RuntimeSession) => session,
    logWarning: (_message: string, fields?: Record<string, unknown>) => warnings.push(fields),
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-stale-focus" }, false)

  assert.equal(launchCalled, false)
  assert.deepEqual(providerRuns, [null])
  assert.deepEqual(warnings, [{
    session_id: "session-stale-focus",
    focused_agent_id: "missing-agent",
  }])
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
    { provider: "opencode", model: "opencode/gpt-5.4", effort: "high" },
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
      model: "opencode/gpt-5.4",
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
    { provider: "opencode", model: "opencode/gpt-5.4", effort: "high" },
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
    getTerminalCommandCatalog: async () => { throw new Error("terminal catalog down") },
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
    "failed to refresh terminal command catalog after attach",
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

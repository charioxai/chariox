import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeAttachment, RuntimeProviderRun, RuntimeSession, SessionHistoryCursorState } from "./cli-types.js"
import { LocalIpcError } from "./ipc.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createBaseDeps } from "./session-lifecycle.test-support.js"
import type { ProviderPreferences } from "./preferences.js"
import type { SessionListEntry } from "./sessions.js"

test("MP-08 / MP-10 OWNER-24 clears the old session before a slow detach", async () => {
  let release!: () => void
  const pending = new Promise<void>(resolve => { release = resolve })
  const { deps, calls } = createBaseDeps({ detachAttachment: () => pending })
  const joining = createSessionLifecycleController(deps as never).attachBinding({ id: "next" }, false)
  try {
    assert.equal(calls.includes("clearTranscript"), true, "previous session output must disappear synchronously")
    assert.equal(calls.includes("clearAgentPaneRuntime"), true)
    assert.equal(calls.includes("requestRender"), true)
  } finally { release() }
  await assert.rejects(joining, /should not be called/)
})

test("attachBinding is a no-op when already attached to the target session", async () => {
  const { deps, calls } = createBaseDeps()
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-1" }, false)

  assert.deepEqual(calls, [])
})

test("MP-08 / MP-10 OWNER-24 resolving the current alias restores the moved-aside attachment", async () => {
  let attachment: { id: string; session_id: string } | null = { id: "old", session_id: "session-1" }
  const { deps } = createBaseDeps({
    attachmentState: () => attachment,
    setAttachmentState: (value: typeof attachment) => { attachment = value },
    detachAttachment: async () => { throw new Error("entered kernel detach") },
    attachToSession: async () => { throw new Error("entered kernel attach") },
    getSessionState: async () => ({ id: "session-1", agents: [], active_provider_run_id: null }),
    hydrateAttachedSessionBinding: async (_id: string, _att: string, session: unknown) => session,
  })
  const controller = createSessionLifecycleController(deps as never)
  const selection = controller.beginSessionSelection({ id: "current-alias" })!
  assert.equal(attachment, null)
  await selection.attach({ id: "session-1" })
  await selection.settle()
  assert.equal((attachment as { id: string } | null)?.id, "old")
  assert.equal(controller.isSessionSelecting(), false)
})

test("session switching clears an attachment the kernel already removed", async () => {
  let attachment: { id: string; session_id: string } | null = { id: "att-stale", session_id: "session-1" }
  const { deps } = createBaseDeps({
    attachmentState: () => attachment,
    detachAttachment: async () => { throw new Error("kernel transport failed: attachment `att-stale` was not found") },
    setAttachmentState: (next: typeof attachment) => { attachment = next },
  })
  await createSessionLifecycleController(deps as never).detachCurrentAttachment()
  assert.equal(attachment, null)
})

test("session switching preserves its attachment on other detach failures", async () => {
  const original = { id: "att-live", session_id: "session-1" }
  let attachment: typeof original | null = original
  const { deps } = createBaseDeps({
    attachmentState: () => attachment,
    detachAttachment: async () => {
      assert.equal(attachment, null, "old-room recovery stops while detach is pending")
      throw new Error("permission denied")
    },
    setAttachmentState: (next: typeof attachment) => { attachment = next },
  })
  await assert.rejects(createSessionLifecycleController(deps as never).detachCurrentAttachment(), /permission denied/)
  assert.equal(attachment, original)
})

test("a completed detach cannot clear a newer attachment", async () => {
  let attachment: { id: string; session_id: string } | null = { id: "att-old", session_id: "session-1" }
  const newer = { id: "att-new", session_id: "session-2" }
  const { deps } = createBaseDeps({
    attachmentState: () => attachment,
    detachAttachment: async () => { attachment = newer },
    setAttachmentState: (next: typeof attachment) => { attachment = next },
  })
  await createSessionLifecycleController(deps as never).detachCurrentAttachment()
  assert.equal(attachment, newer)
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

test("rollbackAttachedSession detaches and clears only the exact target binding", async () => {
  const attachment = { id: "att-managed", session_id: "session-managed" }
  let currentAttachment: typeof attachment | null = attachment
  let currentSessionId = "session-managed"
  const { deps, calls } = createBaseDeps({
    attachmentState: () => currentAttachment,
    sessionState: () => ({ id: currentSessionId }),
    detachAttachment: async (attachmentId: string) => {
      calls.push(`detachAttachment:${attachmentId}`)
    },
    setAttachmentState: (next: typeof attachment | null) => {
      calls.push("setAttachmentState")
      currentAttachment = next
    },
    setSessionState: (session: RuntimeSession) => {
      calls.push("setSessionState")
      currentSessionId = session.id
    },
  })
  const controller = createSessionLifecycleController(deps as never)

  assert.equal(await controller.rollbackAttachedSession("session-other"), false)
  assert.equal(calls.length, 0)

  assert.equal(await controller.rollbackAttachedSession("session-managed"), true)
  assert.equal(currentAttachment, null)
  assert.equal(currentSessionId, "no-session")
  assert.equal(calls[0], "detachAttachment:att-managed")
  assert.ok(calls.includes("refreshWaitingRoomData"))
})

test("attachBinding reattaches and hydrates the attached session before restoring the attached state", async () => {
  const events: string[] = []
  let currentAttachment: RuntimeAttachment | null = null
  const attachedSession: RuntimeSession = {
    id: "session-2",
    project_id: "project-default",
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

  let shownAttachment: unknown = null
  const { deps } = createBaseDeps({
    attachmentState: () => currentAttachment,
    sessionState: () => attachedSession,
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
    setAttachmentState: (value: RuntimeAttachment) => { currentAttachment = value; events.push("setAttachmentState") },
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

  assert.equal(events.filter((event) => event === "setProviderCatalogState").length, 0)
  assert.ok(events.indexOf("setSessionState") < events.indexOf("attachToSession"), "the selected placeholder owns the view before I/O")
  assert.ok(events.indexOf("primeAttachedSessionBinding") < events.indexOf("hydrateAttachedSessionBinding"))
  assert.ok(events.indexOf("syncKernelEventSubscription") < events.indexOf("tryGetProviderRun"))
  assert.ok(events.lastIndexOf("setSessionState") > events.indexOf("hydrateAttachedSessionBinding"), "the final state follows hydration")
  assert.equal(events.at(-1), "setSessionHydratingState")})

test("attachBinding patches the waiting-room session row without listing every session", async () => {
  const appliedSessions: SessionListEntry[][] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    project_id: "project-default",
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
    project_id: "project-default",
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
    project_id: "project-default",
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
  let shownAttachment: unknown = null
  const { deps } = createBaseDeps({
    attachmentState: () => shownAttachment,
    setAttachmentState: (next: unknown) => { shownAttachment = next },
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
    project_id: "project-default",
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

test("attachBinding preserves the actionable vault warning after session hydration", async () => {
  const statusLines: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-vault-locked",
    project_id: "project-default",
    alias: "vault-locked",
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: ["att-vault-locked"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-vault-locked",
    max_agents: 6,
    agents: [{
      id: "agent-vault-locked",
      agent_ref: "agent-vault-locked",
      session_id: "session-vault-locked",
      alias: "agent-vault-locked",
      provider: "codex",
      account_profile: "codex-secondary",
      model: "gpt-5.6-luna",
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
  let shownAttachment: unknown = null
  const { deps } = createBaseDeps({
    attachmentState: () => shownAttachment,
    setAttachmentState: (next: unknown) => { shownAttachment = next },
    attachToSession: async () => ({ id: "att-vault-locked", session_id: "session-vault-locked" }),
    getSessionState: async () => attachedSession,
    launchProviderRun: async () => {
      throw new LocalIpcError(
        "handle kernel response",
        "local transport `credential_vault_locked` failed: Chariox vault is locked",
        "local_transport_error",
        true,
      )
    },
    hydrateAttachedSessionBinding: async () => attachedSession,
    setStatusLine: (message: string) => statusLines.push(message),
  })
  const controller = createSessionLifecycleController(deps as never)

  await controller.attachBinding({ id: "session-vault-locked" }, false)

  assert.equal(statusLines.at(-1), "Chariox vault locked. Run /credential vault manage.")
})

test("attachBinding skips provider launch when existing session exposes no visible agents", async () => {
  const attachedSession: RuntimeSession = {
    id: "session-hidden-focus",
    project_id: "project-default",
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
  let shownAttachment: unknown = null
  const { deps } = createBaseDeps({
    attachmentState: () => shownAttachment,
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
    setAttachmentState: (next: unknown) => { shownAttachment = next },
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
  assert.deepEqual(providerRuns, [null, null], "the placeholder clears the run, the session has none")
})

test("attachBinding skips provider launch when focused agent is stale", async () => {
  const warnings: Array<Record<string, unknown> | undefined> = []
  const providerRuns: Array<RuntimeProviderRun | null> = []
  const attachedSession: RuntimeSession = {
    id: "session-stale-focus",
    project_id: "project-default",
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
  let shownAttachment: unknown = null
  const { deps } = createBaseDeps({
    attachmentState: () => shownAttachment,
    setAttachmentState: (next: unknown) => { shownAttachment = next },
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
  assert.deepEqual(providerRuns, [null, null], "the placeholder clears the run, the session has none")
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
    project_id: "project-default",
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
    project_id: "project-default",
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
  let currentAttachment: RuntimeAttachment | null = null
  const warnings: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    project_id: "project-default",
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
    attachmentState: () => currentAttachment,
    sessionState: () => attachedSession,
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
    setAttachmentState: (value: RuntimeAttachment) => { currentAttachment = value; events.push("setAttachmentState") },
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
    "failed to hydrate attached session after attach",
    "failed to refresh session list after attach",
  ])
})

test("attachBinding synchronizes kernel event subscription immediately after applying attached state", async () => {
  const events: string[] = []
  const attachedSession: RuntimeSession = {
    id: "session-2",
    project_id: "project-default",
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

  let shownAttachment: unknown = null
  const { deps } = createBaseDeps({
    attachmentState: () => shownAttachment,
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
    setAttachmentState: (next: unknown) => { shownAttachment = next; events.push("setAttachmentState") },
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

  assert.deepEqual(events.slice(events.indexOf("setMultiAgentResponseLayout")).slice(0, 15), [
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
    project_id: "project-default",
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

// MP-08 / MP-10 / MP-11 G03: user intent must cancel old-room recovery before transport I/O.
test("detach releases the client binding before the kernel acknowledgement", async () => {
  let attachment: {id: string; session_id: string} | null = {id: "att-1", session_id: "session-1"}
  let acknowledge!: () => void
  const pending = new Promise<void>(resolve => {acknowledge = resolve})
  const {deps} = createBaseDeps({
    attachmentState: () => attachment,
    setAttachmentState: (next: typeof attachment) => {attachment = next},
    detachAttachment: () => pending,
  })
  const controller = createSessionLifecycleController(deps as never)
  const detaching = controller.detachCurrentAttachment()
  const bindingDuringAck = attachment
  const next = {id: "att-2", session_id: "session-2"}
  attachment = next
  acknowledge()
  await detaching
  assert.equal(bindingDuringAck, null)
  assert.equal(attachment, next)
})

// MP-08 / MP-10, PANE-15: model/help catalog latency must not hold the attach command busy.
test("attachment completes while provider and command catalogs are still pending", async () => {
  let releaseProvider!: (value: never) => void
  let releaseCommands!: (value: never) => void
  const provider = new Promise<never>((resolve) => { releaseProvider = resolve })
  const commands = new Promise<never>((resolve) => { releaseCommands = resolve })
  const { deps } = createBaseDeps({
    attachmentState: () => null,
    attachToSession: async () => ({ id: "catalog-attachment", session_id: "session-catalog-latency" }),
    getSessionState: async () => ({ ...createBaseDeps().deps.sessionState(), id: "session-catalog-latency" }),
    getProviderCatalog: () => provider,
    getTerminalCommandCatalog: () => commands,
  })
  const controller = createSessionLifecycleController(deps as never)
  let completed = false
  const attaching = controller.attachBinding({ id: "session-catalog-latency" }, false).then(() => { completed = true })
  for (let i = 0; i < 100; i++) await Promise.resolve()
  try {
    assert.equal(completed, true)
  } finally {
    releaseProvider({} as never)
    releaseCommands({} as never)
    await attaching
  }
})

// MP-08 / MP-10: the previous agent's paging cursor cannot follow a new Room binding.
test("reattach invalidates the previous agent cursor before publishing the new session", async () => {
  let cursor: SessionHistoryCursorState = { agentId: "old-agent", cursor: { before_sequence: 12 } }
  const { deps } = createBaseDeps({
    attachmentState: () => null,
    setNextHistoryCursor: (value: SessionHistoryCursorState) => { cursor = value },
    attachToSession: async () => {
      assert.equal(cursor, null)
      return { id: "new-attachment", session_id: "new-room" }
    },
    getSessionState: async () => ({ ...createBaseDeps().deps.sessionState(), id: "new-room" }),
  })
  await createSessionLifecycleController(deps as never).attachBinding({ id: "new-room" }, false)
})

test("MP-08 / MP-10 an old attach hydration cannot restore the Room after a newer attach", async () => {
  const base = createBaseDeps().deps.sessionState()
  const room = (id: string): RuntimeSession => ({ ...base, id })
  let current = room("initial")
  let binding: RuntimeAttachment | null = null
  let release!: (session: RuntimeSession) => void
  let primed!: () => void
  const oldReachedHydration = new Promise<void>(resolve => { primed = resolve })
  const oldHydration = new Promise<RuntimeSession>(resolve => { release = resolve })
  const { deps } = createBaseDeps({
    attachmentState: () => binding,
    sessionState: () => current,
    setAttachmentState: (value: RuntimeAttachment | null) => { binding = value },
    setSessionState: (value: RuntimeSession) => { current = value },
    detachAttachment: async () => {},
    attachToSession: async (id: string) => ({ id: "att-" + id, session_id: id }),
    getSessionState: async (id: string) => room(id),
    hydrateAttachedSessionBinding: async (id: string) => {
      if (id === "old") { primed(); return oldHydration }
      return room(id)
    },
  })
  const controller = createSessionLifecycleController(deps as never)
  const old = controller.attachBinding({ id: "old" }, false)
  await oldReachedHydration
  const next = controller.attachBinding({ id: "new" }, false)
  assert.equal(current.id, "new")
  release(room("old"))
  await Promise.all([old, next])
  assert.equal(current.id, "new")
  assert.equal((binding as RuntimeAttachment | null)?.session_id, "new")
})

test("MP-08 / MP-10 rejected detach during a switch clears the abandoned hydration", async () => {
  const base = createBaseDeps().deps.sessionState()
  const room = (id: string): RuntimeSession => ({ ...base, id })
  let current = room("initial")
  let binding: RuntimeAttachment | null = null
  let hydrating = false
  let release!: (session: RuntimeSession) => void
  let reached!: () => void
  const pending = new Promise<RuntimeSession>(resolve => { release = resolve })
  const requested = new Promise<void>(resolve => { reached = resolve })
  const { deps } = createBaseDeps({
    attachmentState: () => binding, sessionState: () => current,
    setAttachmentState: (value: RuntimeAttachment | null) => { binding = value },
    setSessionState: (value: RuntimeSession) => { current = value },
    setSessionHydratingState: (value: boolean) => { hydrating = value },
    detachAttachment: async () => { throw new Error("relay lost") },
    attachToSession: async (id: string) => ({ id: "att-" + id, session_id: id }),
    getSessionState: async (id: string) => room(id),
    hydrateAttachedSessionBinding: async (id: string) => { reached(); return pending },
  })
  const controller = createSessionLifecycleController(deps as never)
  const old = controller.attachBinding({ id: "old" }, false)
  await requested
  const next = controller.attachBinding({ id: "new" }, false)
  release(room("old"))
  await assert.rejects(next, /relay lost/)
  await old
  assert.equal(hydrating, false)
  assert.equal((binding as RuntimeAttachment | null)?.session_id, "old", "failed selection restores the retained attachment")
})

// MP-08 / MP-10: a partial Room snapshot has not published its attachment yet.
test("superseded partial attach releases its unpublished attachment", async () => {
  const base = createBaseDeps().deps.sessionState()
  const room = (id: string): RuntimeSession => ({ ...base, id })
  let current = room("initial")
  let binding: RuntimeAttachment | null = null
  let release!: (session: RuntimeSession) => void
  let reached!: () => void
  const pending = new Promise<RuntimeSession>(resolve => { release = resolve })
  const requested = new Promise<void>(resolve => { reached = resolve })
  const detached: string[] = []
  const { deps } = createBaseDeps({
    attachmentState: () => binding,
    sessionState: () => current,
    setAttachmentState: (value: RuntimeAttachment | null) => { binding = value },
    setSessionState: (value: RuntimeSession) => { current = value },
    detachAttachment: async (id: string) => { detached.push(id) },
    attachToSession: async (id: string) => ({ id: "att-" + id, session_id: id }),
    getSessionState: async (id: string) => { if (id === "old") { reached(); return pending } return room(id) },
    hydrateAttachedSessionBinding: async (id: string) => room(id),
  })
  const controller = createSessionLifecycleController(deps as never)
  const old = controller.attachBinding({ id: "old" }, false)
  await requested
  const next = controller.attachBinding({ id: "new" }, false)
  release(room("old"))
  await Promise.all([old, next])
  assert.deepEqual(detached, ["att-old"])
  assert.equal(current.id, "new")
  assert.equal((binding as RuntimeAttachment | null)?.id, "att-new")
})

test("MP-08 / MP-10 OWNER-24 pending selection is hydrating and guarded before reference lookup", () => {
  let hydrating = false
  const { deps } = createBaseDeps({ setSessionHydratingState: (value: boolean) => { hydrating = value } })
  const controller = createSessionLifecycleController(deps as never)
  controller.beginSessionSelection({ id: "next" })
  assert.equal(hydrating, true, "the selected placeholder must not render a waiting room")
  assert.equal(controller.isSessionSelecting(), true)
})

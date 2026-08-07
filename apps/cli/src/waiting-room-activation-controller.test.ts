import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import {
  createWaitingRoomActivationController,
  type WaitingRoomCreateSessionLaunch,
} from "./waiting-room-activation-controller.js"
import type {
  WaitingRoomActivationDecision,
  WaitingRoomControlActivationDecision,
  WaitingRoomCreateSessionDecision,
  WaitingRoomLaunchConfig,
} from "./waiting-room-controller.js"
import type {
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

test("waiting room activation connects detached kernel before control actions", async () => {
  const harness = createHarness({
    kernelConnected: false,
    controlDecision: { action: "cloud" },
  })

  await harness.controller.activate()

  assert.deepEqual(harness.calls, [
    "connectKernel",
    "handleCloudCommand",
  ])
})

test("waiting room activation stages prompt commands from control rows", async () => {
  const harness = createHarness({
    controlDecision: {
      action: "stage-command",
      command: "/workspace /repo",
      message: "edit the workspace path and press Enter",
    },
  })

  await harness.controller.activate()

  assert.equal(harness.promptText, "/workspace /repo")
  assert.deepEqual(harness.calls, [
    "setPromptText",
    "focusPrompt",
    "syncCommandCenter",
    "flash:info:edit the workspace path and press Enter",
  ])
})

test("waiting room activation hydrates a selected kernel inventory", async () => {
  const harness = createHarness({
    controlDecision: {
      action: "browse-kernel",
      kernelId: "kernel-2",
      machineId: "machine-1",
      label: "builder-b",
    },
    browseKernelInventory: async () => 3,
  })

  await harness.controller.activate()

  assert.deepEqual(harness.calls, [
    "browseKernelInventory:kernel-2@machine-1",
    "flash:info:loaded 3 sessions from builder-b",
  ])
})

test("waiting room activation creates and attaches sessions with launch defaults", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "opencode",
    model: "gpt-5.4",
    effort: "high",
    sliceRef: "slice-1",
  }
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: { action: "create", launch },
    accountProfile: "work",
    waitingRoomState: {
      executionMode: "plan",
      permissionLevel: "required",
    },
  })

  await harness.controller.activate()

  assert.deepEqual(harness.createdLaunches, [{
    workspacePath: "/workspace",
    worktreePath: "/worktree",
    launch: {
      provider: "opencode",
      model: "gpt-5.4",
      effort: "high",
      account_profile: "work",
      execution_mode: "plan",
      permission_level: "required",
      workspaceLiveSyncMode: "off",
      sliceRef: "slice-1",
    },
  }])
  assert.deepEqual(harness.attachedSessions, [{
    sessionId: "created-session",
    createdSession: true,
    launch,
  }])
  assert.deepEqual(harness.calls.slice(-4), [
    "updateSlice:slice-1",
    "createSession",
    "attachBinding",
    "flash:info:created session Review in /worktree · slice slice-1 · workspace live sync config default",
  ])
})

test("waiting room activation prepares a selected remote owner before creating the session", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "opencode",
    model: "gpt-5.4",
    effort: "high",
    ownerMachineRef: "machine-1",
    ownerKernelRef: "kernel-1",
  }
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: { action: "create", launch },
    prepareSessionOwnerClient: async () => {},
  })

  await harness.controller.activate()

  assert.deepEqual(harness.calls.slice(0, 3), [
    "prepareSessionOwnerClient:kernel-1",
    "createSession",
    "attachBinding",
  ])
  assert.equal(harness.createdLaunches[0]?.launch.workerKernelRef ?? null, null)
})

test("waiting room activation creates and starts new headed slices before session creation", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "opencode",
    model: "gpt-5.4",
    effort: "high",
    sliceCreate: { displayMode: "headed" },
  }
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: { action: "create", launch },
  })

  await harness.controller.activate()

  assert.deepEqual(harness.createdSlices, [{
    displayMode: "headed",
    workspaceId: "/workspace",
    worktreeId: "/worktree",
    workspaceMount: "/worktree",
  }])
  assert.equal(harness.createdLaunches[0]?.launch.sliceRef, "slice-created")
  assert.deepEqual(harness.calls.slice(-7), [
    "createSlice",
    "updateSlice:slice-created",
    "startSlice:slice-created",
    "updateSlice:slice-created",
    "createSession",
    "attachBinding",
    "flash:info:created session Review in /worktree · slice slice-created · workspace live sync config default",
  ])
})

test("waiting room activation reports explicit created-session live sync mode", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "opencode",
    model: "gpt-5.4",
    effort: "high",
    workspaceLiveSyncMode: "tracked",
  }
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: { action: "create", launch },
    sessionOverrides: { workspace_live_sync_mode: "tracked" },
  })

  await harness.controller.activate()

  assert.deepEqual(harness.calls.slice(-3), [
    "createSession",
    "attachBinding",
    "flash:info:created session Review in /worktree · workspace live sync tracked (selected workspace/worktree only; other repositories unrestricted)",
  ])
  assert.equal(harness.createdLaunches[0]?.launch.workspaceLiveSyncMode, "tracked")
})

test("waiting room prompt bootstrap creates from launch defaults without using focused activation", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "codex",
    model: "gpt-5.4",
    effort: "medium",
  }
  const harness = createHarness({
    kernelConnected: false,
    controlDecision: { action: "cloud" },
    activationDecision: { action: "none" },
    createSessionDecision: { action: "create", launch },
  })

  const session = await harness.controller.startSessionFromWaitingRoomDefaults()

  assert.equal(session.id, "created-session")
  assert.deepEqual(harness.calls, [
    "connectKernel",
    "createSession",
    "attachBinding",
    "flash:info:created session Review in /worktree · workspace live sync config default",
  ])
  assert.deepEqual(harness.attachedSessions, [{
    sessionId: "created-session",
    createdSession: true,
    launch,
  }])
})

test("waiting room activation attaches selected sessions", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "opencode",
    model: "gpt-5.4",
    effort: "medium",
  }
  const joinSession = sessionListEntry("session-2", "Existing")
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: {
      action: "join",
      session: joinSession,
      launch,
    },
  })

  await harness.controller.activate()

  assert.deepEqual(harness.attachedSessions, [{
    sessionId: "session-2",
    createdSession: false,
    launch,
  }])
  assert.deepEqual(harness.calls, [
    "attachBinding",
    "flash:info:attached to session Existing",
  ])
})

test("waiting room activation opens unattached agents as arroba sessions", async () => {
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: {
      action: "import-external-session",
      externalSessionId: "codex:thread-1",
    },
    importSession: runtimeSession("imported-session", "Imported", {
      agent_defaults: {
        provider: "codex",
        model: "codex/gpt-5.4",
        effort: "high",
        account_profile: null,
        execution_mode: "build",
        permission_level: "yolo",
      },
    }),
  })

  await harness.controller.activate()

  assert.deepEqual(harness.importedExternalSessions, ["codex:thread-1"])
  assert.deepEqual(harness.attachedSessions, [{
    sessionId: "imported-session",
    createdSession: true,
    launch: {
      provider: "codex",
      model: "codex/gpt-5.4",
      effort: "high",
    },
  }])
  assert.deepEqual(harness.calls.slice(-3), [
    "importExternalProviderSession:codex:thread-1",
    "attachBinding",
    "flash:info:opened unattached agent codex:thread-1",
  ])
})

test("waiting room activation loads older unattached agent pages", async () => {
  const harness = createHarness({
    controlDecision: { action: "load-older-external-sessions" },
    loadOlderExternalProviderSessions: async () => 2,
  })

  await harness.controller.activate()

  assert.deepEqual(harness.calls, [
    "loadOlderExternalProviderSessions",
    "flash:info:loaded 2 older unattached agents",
  ])
})

test("waiting room activation reports activation failures", async () => {
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: {
      action: "create",
      launch: {
        provider: "opencode",
        model: "gpt-5.4",
        effort: "medium",
      },
    },
    createError: new Error("create failed"),
  })

  await harness.controller.activate()

  assert.deepEqual(harness.warnings, [{
    message: "waiting room activation failed",
    fields: { error: "create failed" },
  }])
  assert.deepEqual(harness.calls.slice(-2), [
    "warn",
    "flash:error:create failed",
  ])
})

function createHarness(options: {
  kernelConnected?: boolean
  controlDecision: WaitingRoomControlActivationDecision
  activationDecision?: WaitingRoomActivationDecision
  createSessionDecision?: WaitingRoomCreateSessionDecision
  accountProfile?: string | null
  createError?: Error
  sessionOverrides?: Partial<RuntimeSession>
  importSession?: RuntimeSession
  loadOlderExternalProviderSessions?: () => Promise<number>
  browseKernelInventory?: (kernelId: string, machineId: string) => Promise<number>
  prepareSessionOwnerClient?: (launch: WaitingRoomLaunchConfig) => Promise<void>
  waitingRoomState?: Partial<WaitingRoomState>
}) {
  const calls: string[] = []
  const attachedSessions: Array<{
    sessionId: string
    createdSession: boolean
    launch: WaitingRoomLaunchConfig
  }> = []
  const createdLaunches: Array<{
    workspacePath: string
    worktreePath: string
    launch: WaitingRoomCreateSessionLaunch
  }> = []
  const createdSlices: Array<{
    displayMode: "headless" | "headed"
    workspaceId: string
    worktreeId: string
    workspaceMount: string
  }> = []
  const warnings: Array<{ message: string; fields: Record<string, unknown> }> = []
  const importedExternalSessions: string[] = []
  let promptText = ""
  const controller = createWaitingRoomActivationController({
    isKernelConnected: () => options.kernelConnected ?? true,
    connectKernel: async () => {
      calls.push("connectKernel")
    },
    getWaitingRoomState: () => (options.waitingRoomState ?? {}) as WaitingRoomState,
    getRemoteState: () => ({} as WaitingRoomRemoteState),
    getWorkspaceTarget: () => "/workspace",
    getWorktreeTarget: () => "/worktree",
    getAvailableSessions: () => [],
    getProviderCatalog: () => ({} as ProviderCatalog),
    getCurrentProvider: () => "opencode" as BackendProviderId,
    getCurrentModel: () => "gpt-5.4",
    getAccountProfile: () => options.accountProfile ?? null,
    handleCloudCommand: async () => {
      calls.push("handleCloudCommand")
    },
    setPromptText: (text) => {
      calls.push("setPromptText")
      promptText = text
    },
    focusPrompt: () => {
      calls.push("focusPrompt")
    },
    syncCommandCenter: () => {
      calls.push("syncCommandCenter")
    },
    openTerminalPairingDialog: async () => {
      calls.push("openTerminalPairingDialog")
    },
    openSessionBrowserDialog: () => {
      calls.push("openSessionBrowserDialog")
    },
    createSession: async (workspacePath, worktreePath, launch) => {
      calls.push("createSession")
      if (options.createError) {
        throw options.createError
      }
      createdLaunches.push({ workspacePath, worktreePath, launch })
      return runtimeSession("created-session", "Review", options.sessionOverrides)
    },
    importExternalProviderSession: async (externalSessionId) => {
      calls.push(`importExternalProviderSession:${externalSessionId}`)
      importedExternalSessions.push(externalSessionId)
      return {
        session: options.importSession ?? runtimeSession("imported-session", "Imported"),
      }
    },
    loadOlderExternalProviderSessions: async () => {
      calls.push("loadOlderExternalProviderSessions")
      return await (options.loadOlderExternalProviderSessions?.() ?? Promise.resolve(0))
    },
    browseKernelInventory: async (kernelId, machineId) => {
      calls.push(`browseKernelInventory:${kernelId}@${machineId}`)
      return await (options.browseKernelInventory?.(kernelId, machineId) ?? Promise.resolve(0))
    },
    createSlice: async (slice) => {
      calls.push("createSlice")
      createdSlices.push({
        displayMode: slice.displayMode,
        workspaceId: slice.workspaceId,
        worktreeId: slice.worktreeId,
        workspaceMount: slice.workspaceMount,
      })
      return sliceRecord("slice-created", slice.displayMode)
    },
    startSlice: async (sliceRef) => {
      calls.push(`startSlice:${sliceRef}`)
      return sliceRecord(sliceRef, "headed")
    },
    updateSlices: (slice) => {
      calls.push(`updateSlice:${slice.id}`)
    },
    ...(options.prepareSessionOwnerClient
      ? {
        prepareSessionOwnerClient: async (launch: WaitingRoomLaunchConfig) => {
          calls.push(`prepareSessionOwnerClient:${launch.ownerKernelRef ?? "local"}`)
          await options.prepareSessionOwnerClient?.(launch)
        },
      }
      : {}),
    attachBinding: async (session, createdSession, launch) => {
      calls.push("attachBinding")
      attachedSessions.push({ sessionId: session.id, createdSession, launch })
    },
    flashFooter: (message, tone) => {
      calls.push(`flash:${tone}:${message}`)
    },
    warn: (message, fields) => {
      calls.push("warn")
      warnings.push({ message, fields })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    deriveControlDecision: () => options.controlDecision,
    deriveActivationDecision: () => options.activationDecision ?? { action: "none" },
    deriveCreateSessionDecision: () => options.createSessionDecision ?? { action: "error", message: "not configured" },
  })
  return {
    get promptText() {
      return promptText
    },
    calls,
    attachedSessions,
    createdLaunches,
    createdSlices,
    importedExternalSessions,
    warnings,
    controller,
  }
}

function runtimeSession(id: string, alias: string | null, overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id,
    alias,
    workspace_id: "/workspace",
    worktree_id: "/worktree",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "opencode",
      model: "gpt-5.4",
      effort: "medium",
      account_profile: null,
      execution_mode: "build",
      permission_level: "yolo",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function sliceRecord(id: string, displayMode: "headless" | "headed") {
  return {
    id,
    name: id,
    owner_kernel_id: "kernel",
    owner_machine_id: "machine",
    backend: "local_docker" as const,
    os: "linux",
    display_mode: displayMode,
    status: "running" as const,
    workspace_mount: "/worktree",
    workspace_id: "/workspace",
    worktree_id: "/worktree",
    worker_kernel_ref: `slice:${id}`,
    worker_kernel_id: `kernel-${id}`,
    worker_machine_id: `machine-${id}`,
    providers: [],
    provider_auth: [],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
}

function sessionListEntry(id: string, alias: string | null): SessionListEntry {
  return {
    id,
    alias,
    workspace_id: "/workspace",
    worktree_id: "/worktree",
    created_at_ms: 1,
    status: "Active",
    connected_cli_count: 0,
  }
}

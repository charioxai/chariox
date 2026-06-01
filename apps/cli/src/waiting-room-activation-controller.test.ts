import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, SliceRecord } from "./cli-types.js"
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
      execution_mode: "build",
      permission_level: "yolo",
      workspaceLiveSyncMode: "off",
      sliceRef: "slice-1",
    },
  }])
  assert.deepEqual(harness.attachedSessions, [{
    sessionId: "created-session",
    createdSession: true,
    launch,
  }])
  assert.deepEqual(harness.calls.slice(-5), [
    "importAuth:slice-1:all",
    "updateSlice:slice-1",
    "createSession",
    "attachBinding",
    "flash:info:created session Review in /worktree · slice slice-1 · screen http://127.0.0.1:45503/vnc.html · workspace live sync config default",
  ])
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
  assert.deepEqual(harness.calls.slice(-9), [
    "createSlice",
    "updateSlice:slice-created",
    "startSlice:slice-created",
    "updateSlice:slice-created",
    "importAuth:slice-created:all",
    "updateSlice:slice-created",
    "createSession",
    "attachBinding",
    "flash:info:created session Review in /worktree · slice slice-created · screen http://127.0.0.1:45503/vnc.html · workspace live sync config default",
  ])
})

test("waiting room activation blocks slice launches when selected provider auth is missing", async () => {
  const launch: WaitingRoomLaunchConfig = {
    provider: "claude",
    model: "claude-sonnet-4-6",
    effort: "high",
    sliceRef: "slice-1",
  }
  const harness = createHarness({
    controlDecision: { action: "none" },
    activationDecision: { action: "create", launch },
    importedProviderAuth: [{ provider: "codex", state: "configured" }],
  })

  await harness.controller.activate()

  assert.equal(harness.createdLaunches.length, 0)
  assert.deepEqual(harness.calls.slice(-3), [
    "updateSlice:slice-1",
    "warn",
    "flash:error:slice slice-1 is missing claude auth; run /slice auth import slice-1 all or /slice auth login slice-1 claude",
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
  importedProviderAuth?: SliceRecord["provider_auth"]
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
  let promptText = ""
  const controller = createWaitingRoomActivationController({
    isKernelConnected: () => options.kernelConnected ?? true,
    connectKernel: async () => {
      calls.push("connectKernel")
    },
    getWaitingRoomState: () => ({} as WaitingRoomState),
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
    importSliceProviderAuth: async (sliceRef, provider) => {
      calls.push(`importAuth:${sliceRef}:${provider}`)
      return {
        slice: sliceRecord(sliceRef, "headed", {
          provider_auth: options.importedProviderAuth ?? [
            { provider: "opencode", state: "configured" },
            { provider: "codex", state: "configured" },
            { provider: "claude", state: "configured" },
          ],
        }),
        provider,
        status: "imported",
      }
    },
    updateSlices: (slice) => {
      calls.push(`updateSlice:${slice.id}`)
    },
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

function sliceRecord(
  id: string,
  displayMode: "headless" | "headed",
  overrides: Partial<SliceRecord> = {},
): SliceRecord {
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
    display_endpoint: displayMode === "headed" ? {
      slice_id: id,
      kind: "novnc" as const,
      url: "http://127.0.0.1:45503/vnc.html",
      access: "local" as const,
      capabilities: [],
    } : null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
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

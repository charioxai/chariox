import assert from "node:assert/strict"
import test from "node:test"

import { createCliAutomationActionHandler } from "./cli-automation-handler.js"
import type { CliOptions, ExternalProviderSessionRecord, RuntimeSession } from "./cli-types.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"

test("automation action handler switches attached workspace screens through app deps", async () => {
  let screen: WorkspaceScreenMode = "agents"
  let rebuilt = false
  let layoutApplied = false
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    snapshot: () => ({ screen }),
    isAttached: () => true,
    setWorkspaceScreenMode: (next) => {
      screen = next
    },
    rebuildTranscript: () => {
      rebuilt = true
    },
    applyResponseLayout: () => {
      layoutApplied = true
    },
  })

  const result = await handler({ action: "switch_screen", screen: "workflow" })

  assert.deepEqual(result, { screen: "workflow" })
  assert.equal(rebuilt, true)
  assert.equal(layoutApplied, true)
})

test("automation action handler rejects screen switching while detached", async () => {
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    isAttached: () => false,
  })

  await assert.rejects(
    handler({ action: "switch_screen", screen: "workflow" }),
    /cannot switch screen without an attached session/,
  )
})

test("automation action handler waits until snapshot filters match", async () => {
  let attempts = 0
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    snapshot: () => {
      attempts += 1
      return { shell: { entries: Array.from({ length: attempts >= 3 ? 2 : 1 }, (_entry, index) => ({ id: index })) } }
    },
    sleep: async () => {},
  })

  const result = await handler({
    action: "wait_for",
    shellEntryCount: 2,
    intervalMs: 1,
    timeoutMs: 100,
  })

  assert.deepEqual(result, { shell: { entries: [{ id: 0 }, { id: 1 }] } })
  assert.equal(attempts, 3)
})

test("automation action handler sets focused interaction custom reply", async () => {
  const writes: Array<{ interactionId: string; reply: string }> = []
  const editing: Array<{ interactionId: string; editing: boolean }> = []
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    focusedAgentId: () => "agent-1",
    snapshot: () => ({
      interactions: [
        { id: "interaction-1", agentId: "agent-1" },
      ],
    }),
    setInteractionCustomReply: (interactionId, reply) => {
      writes.push({ interactionId, reply })
    },
    setInteractionCustomEditing: (interactionId, next) => {
      editing.push({ interactionId, editing: next })
    },
  })

  await handler({ action: "interaction_custom_reply", reply: "vault-passphrase", editing: true })

  assert.deepEqual(writes, [{ interactionId: "interaction-1", reply: "vault-passphrase" }])
  assert.deepEqual(editing, [{ interactionId: "interaction-1", editing: true }])
})

test("automation prompt submit does not relaunch when the session has an active provider run", async () => {
  let promptText = ""
  let submitted = false
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    isAttached: () => true,
    sessionState: () => ({
      id: "session-1",
      active_provider_run_id: "provider-run-1",
    }) as RuntimeSession,
    setPromptText: (value) => {
      promptText = value
    },
    submitPrompt: async () => {
      submitted = true
    },
    snapshot: () => ({ promptText, submitted }),
  })

  const result = await handler({ action: "submit_prompt", prompt: "hello" })

  assert.deepEqual(result, { promptText: "hello", submitted: true })
})

test("automation action handler activates a selected unattached agent", async () => {
  let waitingRoomState: WaitingRoomState = waitingRoomFixture()
  let activated = false
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    waitingRoomState: () => waitingRoomState,
    setWaitingRoomState: (next) => {
      waitingRoomState = next
    },
    externalProviderSessionsState: () => [
      externalSession("opencode:first", { last_modified_at_ms: 100 }),
      externalSession("codex:second", { last_modified_at_ms: 200 }),
    ],
    activateWaitingRoom: async () => {
      activated = true
    },
    snapshot: () => ({
      waitingRoomState,
      activated,
    }),
  })

  const result = await handler({ action: "activate_unattached_agent", externalSessionId: "codex:second" })

  assert.equal(activated, true)
  assert.deepEqual(result, {
    waitingRoomState: {
      ...waitingRoomState,
      focus: "external-session",
      externalSessionIndex: 0,
    },
    activated: true,
  })
})

test("automation action handler indexes unattached agents in shared projected order", async () => {
  let waitingRoomState: WaitingRoomState = waitingRoomFixture()
  let activated = false
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    waitingRoomState: () => waitingRoomState,
    setWaitingRoomState: (next) => {
      waitingRoomState = next
    },
    externalProviderSessionsState: () => [
      externalSession("codex:old", { last_modified_at_ms: 100 }),
      externalSession("claude:recent", { last_modified_at_ms: 200 }),
    ],
    activateWaitingRoom: async () => {
      activated = true
    },
    snapshot: () => ({
      waitingRoomState,
      activated,
    }),
  })

  const result = await handler({ action: "activate_unattached_agent", externalSessionIndex: 0 })

  assert.equal(activated, true)
  assert.deepEqual(result, {
    waitingRoomState: {
      ...waitingRoomState,
      focus: "external-session",
      externalSessionIndex: 0,
    },
    activated: true,
  })
})

test("automation action handler sets waiting room launch placement", async () => {
  let waitingRoomState: WaitingRoomState = waitingRoomFixture()
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    waitingRoomState: () => waitingRoomState,
    setWaitingRoomState: (next) => {
      waitingRoomState = next
    },
    snapshot: () => ({ waitingRoomState }),
  })

  const result = await handler({
    action: "set_waiting_room_launch",
    machineRef: "machine-peer",
    kernelRef: "kernel-peer",
    providerId: "dev-stub",
    modelId: "dev-stub/test",
    effort: "low",
    focus: "new",
  })

  assert.deepEqual(result, {
    waitingRoomState: {
      ...waitingRoomFixture(),
      selectedMachineRef: "machine-peer",
      selectedKernelRef: "kernel-peer",
      providerId: "dev-stub",
      modelId: "dev-stub/test",
      effort: "low",
      focus: "new",
    },
  })
})

test("automation connect action refreshes waiting room when already connected", async () => {
  let refreshed = false
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    kernelConnected: () => true,
    refreshWaitingRoomData: async () => {
      refreshed = true
    },
    snapshot: () => ({ refreshed }),
  })

  const result = await handler({ action: "connect_detached_kernel" })

  assert.deepEqual(result, { refreshed: true })
})

test("automation action handler toggles an agent pane blob when agentId is provided", async () => {
  const toggles: Array<{ agentId: string; entryId: number; collapsed: boolean }> = []
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    toggleAgentPaneBlob: (agentId, entryId, collapsed) => {
      toggles.push({ agentId, entryId, collapsed })
    },
    snapshot: () => ({ toggles }),
  })

  const result = await handler({
    action: "toggle_blob",
    agentId: "agent-1",
    entryId: 42,
    collapsed: false,
  })

  assert.deepEqual(toggles, [{ agentId: "agent-1", entryId: 42, collapsed: false }])
  assert.deepEqual(result, { toggles })
})

test("automation action handler toggles an agent pane turn when agentId is provided", async () => {
  const toggles: Array<{ agentId: string; turnId: number; toggleEntryId?: number }> = []
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    toggleAgentPaneTurn: (agentId, turnId, toggleEntryId) => {
      toggles.push(toggleEntryId === undefined
        ? { agentId, turnId }
        : { agentId, turnId, toggleEntryId })
    },
    snapshot: () => ({ toggles }),
  })

  const result = await handler({
    action: "toggle_turn",
    agentId: "agent-1",
    turnId: 7,
    entryId: 99,
  })

  assert.deepEqual(toggles, [{ agentId: "agent-1", turnId: 7, toggleEntryId: 99 }])
  assert.deepEqual(result, { toggles })
})

test("automation action handler runs queued prompt action for selected strip item", async () => {
  const actions: Array<{ promptId: string; action: "steer" | "cancel" }> = []
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    focusedAgentId: () => "agent-1",
    queuedPromptStripItemsForAgent: (agentId) => agentId === "agent-1"
      ? [queuedPromptItem("prompt-1"), queuedPromptItem("prompt-2")]
      : [],
    selectedQueuedPromptIndexForAgent: () => 1,
    onQueuedPromptAction: async (item, action) => {
      actions.push({ promptId: item.promptId, action })
    },
    snapshot: () => ({ actions }),
  })

  const result = await handler({
    action: "queued_prompt_action",
    queuedPromptAction: "cancel",
  })

  assert.deepEqual(actions, [{ promptId: "prompt-2", action: "cancel" }])
  assert.deepEqual(result, { actions })
})

test("automation action handler can target queued prompt action by prompt id", async () => {
  const actions: Array<{ promptId: string; action: "steer" | "cancel" }> = []
  const handler = createCliAutomationActionHandler({
    ...baseDeps(),
    queuedPromptStripItemsForAgent: (agentId) => agentId === "agent-1"
      ? [queuedPromptItem("prompt-1"), queuedPromptItem("prompt-2")]
      : [],
    onQueuedPromptAction: (item, action) => {
      actions.push({ promptId: item.promptId, action })
    },
    snapshot: () => ({ actions }),
  })

  await handler({
    action: "queued_prompt_action",
    agentId: "agent-1",
    promptId: "prompt-1",
    queuedPromptAction: "steer",
  })

  assert.deepEqual(actions, [{ promptId: "prompt-1", action: "steer" }])
})

function baseDeps() {
  return {
    client: null as never,
    options: { provider: "opencode", model: "default", effort: "" } as CliOptions,
    appLogger: null,
    snapshot: () => ({}),
    isAttached: () => false,
    kernelConnected: () => false,
    workflowScreenActive: () => false,
    setWorkspaceScreenMode: (_screen: WorkspaceScreenMode) => {},
    rebuildTranscript: () => {},
    applyResponseLayout: () => {},
    showWorkflowScreen: () => {},
    submitWorkspaceShellCommand: async () => null,
    attachmentState: () => null,
    sessionState: () => ({ id: "session-1" }) as RuntimeSession,
    focusedAgentId: () => null,
    setPromptText: () => {},
    submitPrompt: async () => {},
    activateWaitingRoom: async () => {},
    connectDetachedKernelFromWaitingRoom: async () => {},
    refreshWaitingRoomData: async () => {},
    submitFocusedInteractionChoice: async () => {},
    cycleFocusedInteractionChoice: () => {},
    setInteractionCustomReply: () => {},
    setInteractionCustomEditing: () => {},
    toggleTurn: () => {},
    toggleBlob: () => {},
    queuedPromptStripItemsForAgent: () => [],
    selectedQueuedPromptIndexForAgent: () => 0,
    onQueuedPromptAction: () => {},
    restoreTerminalAndExit: async () => {},
    waitingRoomState: () => waitingRoomFixture(),
    setWaitingRoomState: () => {},
    externalProviderSessionsState: () => [],
  }
}

function queuedPromptItem(promptId: string) {
  return {
    promptId,
    agentId: "agent-1",
    sourceAttachmentId: null,
    prompt: `queued prompt ${promptId}`,
    status: "queued",
    attachmentCount: 0,
    steerDisabled: false,
    canSteer: true,
    canCancel: true,
    steerDisabledReason: null,
    cancelDisabledReason: null,
  }
}

function externalSession(
  id: string,
  overrides: Partial<ExternalProviderSessionRecord> = {},
): ExternalProviderSessionRecord {
  return {
    external_session_id: id,
    provider: id.split(":")[0] ?? "codex",
    provider_session_id: id,
    title: id,
    title_source: "provider",
    first_prompt_preview: id,
    created_at_ms: 1,
    last_modified_at_ms: 2,
    capabilities: {
      can_read_history: true,
    },
    ...overrides,
  }
}

function waitingRoomFixture(): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    externalSessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "/repo",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "default",
    effort: "",
    themeId: "dark",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
  }
}

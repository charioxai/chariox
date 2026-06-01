import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord } from "./cli-types.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type { WaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import {
  createWaitingRoomLifecycleActionController,
  type WaitingRoomLifecycleActionControllerDeps,
} from "./waiting-room-lifecycle-action-controller.js"

test("waiting room lifecycle action waits for confirmation", async () => {
  const harness = createHarness({
    confirmationController: confirmationController("await"),
  })

  await harness.controller.applyAction("archive")

  assert.deepEqual(harness.archivedSessionIds(), [])
  assert.equal(harness.footerMessages().at(-1)?.message, "confirm first")
  assert.equal(harness.footerMessages().at(-1)?.tone, "info")
})

test("waiting room lifecycle action archives the selected session", async () => {
  const harness = createHarness()

  await harness.controller.applyAction("archive")

  assert.deepEqual(harness.archivedSessionIds(), ["session-1"])
  assert.deepEqual(harness.availableSessions().map((session) => session.id), ["session-2"])
  assert.equal(harness.invalidateCount(), 1)
  assert.equal(harness.refreshCount(), 1)
  assert.equal(harness.reconciledStates().at(-1)?.focus, "session")
  assert.equal(harness.footerMessages().at(-1)?.message, "archived session one")
})

test("waiting room lifecycle action deletes all sessions and closes the session browser", async () => {
  const harness = createHarness({
    state: waitingRoomState({ focus: "join-sessions" }),
    sessionBrowserOpen: true,
  })

  await harness.controller.applyAction("delete")

  assert.deepEqual(harness.deletedSessionIds(), ["session-1", "session-2"])
  assert.deepEqual(harness.availableSessions(), [])
  assert.equal(harness.closedSessionBrowserCount(), 1)
  assert.equal(harness.reconciledStates().at(-1)?.focus, "new")
  assert.equal(harness.footerMessages().at(-1)?.message, "deleted 2 sessions")
})

test("waiting room lifecycle action deletes inactive machines and child kernels", async () => {
  const harness = createHarness({
    state: waitingRoomState({ focus: "machine" }),
    remoteMachines: [
      remoteMachine("machine-1", { online: false, kernel_count: 1 }),
      remoteMachine("machine-2", { online: true, kernel_count: 1 }),
    ],
    remoteKernels: [
      remoteKernel("kernel-1", "machine-1"),
      remoteKernel("kernel-2", "machine-2"),
    ],
  })

  await harness.controller.applyAction("delete")

  assert.deepEqual(harness.forgottenMachineIds(), ["machine-1"])
  assert.deepEqual(harness.remoteMachines().map((machine) => machine.machine_id), ["machine-2"])
  assert.deepEqual(harness.remoteKernels().map((kernel) => kernel.kernel_id), ["kernel-2"])
  assert.equal(harness.footerMessages().at(-1)?.message, "deleted machine machine-1")
})

test("waiting room lifecycle action hides inactive kernels", async () => {
  const harness = createHarness({
    state: waitingRoomState({ focus: "remote-kernel" }),
    remoteKernels: [
      remoteKernel("kernel-1", "machine-1", {
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
      }),
      remoteKernel("kernel-2", "machine-1"),
    ],
  })

  await harness.controller.applyAction("delete")

  assert.deepEqual(harness.hiddenKernelIds(), ["kernel-1"])
  assert.deepEqual(harness.remoteKernels().map((kernel) => kernel.kernel_id), ["kernel-2"])
  assert.equal(harness.refreshCount(), 0)
  assert.equal(harness.footerMessages().at(-1)?.message, "deleted kernel kernel-1")
})

test("waiting room lifecycle action deletes idle slices", async () => {
  const harness = createHarness({
    state: waitingRoomState({ focus: "slice-entry", sliceIndex: 0 }),
    slices: [
      slice("slice-1", "linux-dev"),
      slice("slice-2", "other-dev"),
    ],
  })

  await harness.controller.applyAction("delete")

  assert.deepEqual(harness.deletedSliceIds(), ["slice-1"])
  assert.deepEqual(harness.slices().map((candidate) => candidate.id), ["slice-2"])
  assert.equal(harness.invalidateCount(), 1)
  assert.equal(harness.refreshCount(), 1)
  assert.equal(harness.footerMessages().at(-1)?.message, "deleted slice linux-dev")
})

test("waiting room lifecycle action blocks active slice deletion before confirmation", async () => {
  const harness = createHarness({
    state: waitingRoomState({ focus: "slice-entry", sliceIndex: 0 }),
    slices: [
      slice("slice-1", "busy-dev", { agent_ids: ["agent-1"] }),
    ],
  })

  await harness.controller.applyAction("delete")

  assert.deepEqual(harness.deletedSliceIds(), [])
  assert.equal(harness.footerMessages().at(-1)?.message, "slice busy-dev has 1 active agent")
})

function createHarness(options: {
  state?: WaitingRoomState
  sessions?: SessionListEntry[]
  remoteMachines?: WaitingRoomLifecycleActionControllerDeps["getRemoteMachines"] extends () => infer T ? T : never
  remoteKernels?: WaitingRoomLifecycleActionControllerDeps["getRemoteKernels"] extends () => infer T ? T : never
  slices?: SliceRecord[]
  confirmationController?: WaitingRoomLifecycleConfirmationController
  sessionBrowserOpen?: boolean
} = {}) {
  let availableSessions = options.sessions ?? [
    session("session-1", "one"),
    session("session-2", "two"),
  ]
  let remoteMachines = options.remoteMachines ?? []
  let remoteKernels = options.remoteKernels ?? []
  let slices = options.slices ?? []
  const state = options.state ?? waitingRoomState({ focus: "session" })
  const archivedSessionIds: string[] = []
  const deletedSessionIds: string[] = []
  const forgottenMachineIds: string[] = []
  const hiddenKernelIds: string[] = []
  const deletedSliceIds: string[] = []
  const reconciledStates: WaitingRoomState[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  let invalidateCount = 0
  let refreshCount = 0
  let closedSessionBrowserCount = 0

  const controller = createWaitingRoomLifecycleActionController({
    isKernelConnected: () => true,
    connectDetachedKernel: async () => {},
    getWaitingRoomState: () => state,
    getRemoteState: () => ({
      machines: remoteMachines,
      kernels: remoteKernels,
      slices,
    }),
    getAvailableSessions: () => availableSessions,
    setAvailableSessions: (sessions) => {
      availableSessions = sessions
    },
    getProviderCatalog: () => fallbackProviderCatalog(),
    getWorkspaceTarget: () => "/workspace",
    confirmationController: options.confirmationController ?? confirmationController("confirmed"),
    archiveSessionById: async (sessionId) => {
      archivedSessionIds.push(sessionId)
      return { id: sessionId, alias: availableSessions.find((candidate) => candidate.id === sessionId)?.alias ?? null }
    },
    deleteSessionByRef: async (sessionId) => {
      deletedSessionIds.push(sessionId)
      return { id: sessionId, alias: availableSessions.find((candidate) => candidate.id === sessionId)?.alias ?? null }
    },
    forgetRemoteMachine: async (machineId) => {
      forgottenMachineIds.push(machineId)
      return { machine_id: machineId }
    },
    getRemoteMachines: () => remoteMachines,
    setRemoteMachines: (machines) => {
      remoteMachines = machines
    },
    getRemoteKernels: () => remoteKernels,
    setRemoteKernels: (kernels) => {
      remoteKernels = kernels
    },
    getSlices: () => slices,
    setSlices: (nextSlices) => {
      slices = nextSlices
    },
    deleteSlice: async (sliceRef) => {
      deletedSliceIds.push(sliceRef)
      return slices.find((candidate) => candidate.id === sliceRef) ?? slice(sliceRef, sliceRef)
    },
    hideRemoteKernel: (kernelId) => {
      hiddenKernelIds.push(kernelId)
    },
    invalidateInventory: () => {
      invalidateCount += 1
    },
    reconcileWaitingRoom: (nextState) => {
      reconciledStates.push(nextState)
    },
    refreshWaitingRoomData: async () => {
      refreshCount += 1
    },
    sessionBrowserOpen: () => options.sessionBrowserOpen ?? false,
    closeSessionBrowserDialog: () => {
      closedSessionBrowserCount += 1
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    availableSessions: () => availableSessions,
    remoteMachines: () => remoteMachines,
    remoteKernels: () => remoteKernels,
    slices: () => slices,
    archivedSessionIds: () => archivedSessionIds,
    deletedSessionIds: () => deletedSessionIds,
    forgottenMachineIds: () => forgottenMachineIds,
    hiddenKernelIds: () => hiddenKernelIds,
    deletedSliceIds: () => deletedSliceIds,
    reconciledStates: () => reconciledStates,
    footerMessages: () => footerMessages,
    invalidateCount: () => invalidateCount,
    refreshCount: () => refreshCount,
    closedSessionBrowserCount: () => closedSessionBrowserCount,
  }
}

function confirmationController(mode: "confirmed" | "await"): WaitingRoomLifecycleConfirmationController {
  return {
    confirm: () => mode === "confirmed"
      ? { action: "confirmed", target: { kind: "session", id: "session-1", label: "session one", verb: "archive" } }
      : { action: "await-confirmation", target: { kind: "session", id: "session-1", label: "session one", verb: "archive" }, message: "confirm first", tone: "info" },
    clear: () => {},
    pending: () => null,
  }
}

function session(id: string, alias: string): SessionListEntry {
  return {
    id,
    alias,
    status: "Created",
    worktree_id: "/workspace/tree",
  }
}

function waitingRoomState(overrides: Partial<WaitingRoomState>): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "current",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "medium",
    themeId: "system",
    introStep: 0,
    keyState: {
      up: false,
      down: false,
      left: false,
      right: false,
    },
    ...overrides,
  }
}

function slice(id: string, name: string, overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id,
    name,
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker",
    os: "linux",
    status: "stopped",
    workspace_mount: "/workspace",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    worker_kernel_ref: `slice:${id}`,
    worker_kernel_id: `kernel-${id}`,
    worker_machine_id: `machine-${id}`,
    providers: ["codex"],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function remoteMachine(
  machineId: string,
  overrides: Partial<WaitingRoomLifecycleActionControllerDeps["getRemoteMachines"] extends () => Array<infer T> ? T : never> = {},
) {
  return {
    machine_id: machineId,
    display_name: machineId,
    trust_status: "approved" as const,
    online: true,
    pending: false,
    kernel_count: 0,
    ...overrides,
  }
}

function remoteKernel(
  kernelId: string,
  machineId: string,
  overrides: Partial<WaitingRoomLifecycleActionControllerDeps["getRemoteKernels"] extends () => Array<infer T> ? T : never> = {},
) {
  return {
    kernel_id: kernelId,
    machine_id: machineId,
    relay_alias: kernelId,
    accepting_remote_leases: false,
    leased_agent_count: 0,
    local_session_count: 0,
    ...overrides,
  }
}

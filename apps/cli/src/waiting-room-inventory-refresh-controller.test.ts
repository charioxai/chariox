import assert from "node:assert/strict"
import test from "node:test"

import type { SliceRecord, WaitingRoomPublicSessionSummary } from "./cli-types.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState, type WaitingRoomState } from "./waiting-room.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
  WaitingRoomInventory,
} from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"

test("waiting room inventory refresh is idle while kernel is disconnected", async () => {
  const harness = createHarness({ connected: false })

  await harness.controller.refreshNow()

  assert.equal(harness.inventoryCalls(), 0)
  assert.equal(harness.reconcileCount(), 0)
})

test("waiting room inventory refresh applies snapshots and filters hidden inactive kernels", async () => {
  const harness = createHarness({
    hiddenKernelIds: new Set(["kernel-hidden", "kernel-busy"]),
    snapshots: [inventory("v1", {
      remoteKernels: [
        kernel("kernel-hidden"),
        kernel("kernel-busy", { leased_agent_count: 1 }),
        kernel("kernel-visible"),
      ],
    })],
  })

  await harness.controller.refreshNow()

  assert.equal(harness.inventoryStatus(), "ready")
  assert.deepEqual(harness.remoteKernels().map((kernel) => kernel.kernel_id), ["kernel-busy", "kernel-visible"])
  assert.equal(harness.reconcileCount(), 1)
})

test("waiting room inventory refresh reconciles unchanged versions without replacing state", async () => {
  const harness = createHarness({
    snapshots: [
      inventory("v1", { sessions: [session("session-1")] }),
      inventory("v1", { sessions: [session("session-2")] }),
    ],
  })

  await harness.controller.refreshNow()
  await harness.controller.refreshNow()

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-1"])
  assert.equal(harness.reconcileCount(), 2)
})

test("waiting room inventory refresh reports failures", async () => {
  const harness = createHarness({
    getInventory: async () => {
      throw new Error("inventory down")
    },
  })

  await harness.controller.refreshNow()

  assert.equal(harness.inventoryStatus(), "error")
  assert.equal(harness.warnings().at(-1)?.message, "waiting room inventory refresh failed")
  assert.equal(harness.reconcileCount(), 0)
})

test("waiting room inventory refresh coalesces concurrent refreshes", async () => {
  const pendingInventory = deferred<WaitingRoomInventory>()
  const harness = createHarness({
    getInventory: () => pendingInventory.promise,
  })

  const first = harness.controller.refresh()
  const second = harness.controller.refresh()
  pendingInventory.resolve(inventory("v1"))
  await Promise.all([first, second])

  assert.equal(harness.inventoryCalls(), 1)
})

function createHarness(options: {
  connected?: boolean
  hiddenKernelIds?: Set<string>
  snapshots?: WaitingRoomInventory[]
  getInventory?: () => Promise<WaitingRoomInventory>
} = {}) {
  const catalog = fallbackProviderCatalog()
  const hiddenKernelIds = options.hiddenKernelIds ?? new Set<string>()
  let inventoryStatus: "loading" | "ready" | "error" = "loading"
  let waitingRoomState = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "medium")
  let availableSessions: SessionListEntry[] = []
  let remoteKernels: RemoteKernelView[] = []
  let inventoryCalls = 0
  let reconcileCount = 0
  const warnings: Array<{ message: string; fields: Record<string, unknown> }> = []
  const snapshots = [...(options.snapshots ?? [inventory("v1")])]

  const controller = createWaitingRoomInventoryRefreshController({
    isKernelConnected: () => options.connected ?? true,
    getInventoryStatus: () => inventoryStatus,
    setInventoryStatus: (status) => {
      inventoryStatus = status
    },
    getWaitingRoomState: () => waitingRoomState,
    getInventory: async () => {
      inventoryCalls += 1
      if (options.getInventory) {
        return options.getInventory()
      }
      return snapshots.shift() ?? inventory("fallback")
    },
    isKernelHidden: (kernelId) => hiddenKernelIds.has(kernelId),
    setAvailableSessions: (sessions) => {
      availableSessions = sessions
    },
    setRelayStatus: () => {},
    setRemoteMachines: () => {},
    setRemoteKernels: (kernels) => {
      remoteKernels = kernels
    },
    setTerminals: () => {},
    setSlices: () => {},
    reconcileWaitingRoom: (state) => {
      waitingRoomState = state
      reconcileCount += 1
    },
    warn: (message, fields) => {
      warnings.push({ message, fields })
    },
  })

  return {
    controller,
    inventoryCalls: () => inventoryCalls,
    inventoryStatus: () => inventoryStatus,
    availableSessions: () => availableSessions,
    remoteKernels: () => remoteKernels,
    reconcileCount: () => reconcileCount,
    warnings: () => warnings,
  }
}

function inventory(
  inventoryVersion: string,
  overrides: Partial<WaitingRoomInventory> = {},
): WaitingRoomInventory {
  return {
    inventoryVersion,
    sessions: [],
    relayStatus: { configured: false } as RelayStatusView,
    remoteMachines: [],
    remoteKernels: [],
    terminals: [] as TerminalView[],
    slices: [] as SliceRecord[],
    ...overrides,
  }
}

function session(id: string): WaitingRoomPublicSessionSummary {
  return {
    id,
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: "Created",
    created_at_ms: 1,
    connected_cli_count: 0,
  }
}

function kernel(id: string, overrides: Partial<RemoteKernelView> = {}): RemoteKernelView {
  return {
    kernel_id: id,
    machine_id: "machine-1",
    accepting_remote_leases: false,
    leased_agent_count: 0,
    local_session_count: 0,
    ...overrides,
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

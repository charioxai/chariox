import assert from "node:assert/strict"
import test from "node:test"

import type { ExternalProviderSessionRecord, SliceRecord, WaitingRoomPublicSessionSummary } from "./cli-types.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
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

test("waiting room inventory row patch applies first row set without fetching", () => {
  const harness = createHarness({ connected: true })

  harness.controller.applyRowsChanged({
    inventoryVersion: "v1",
    sessions: [session("session-1"), session("session-2")],
    removedSessionIds: [],
  })

  assert.equal(harness.inventoryCalls(), 0)
  assert.equal(harness.inventoryStatus(), "ready")
  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-1", "session-2"])
  assert.equal(harness.reconcileCount(), 1)
})

test("waiting room inventory row patch merges changes and removals", () => {
  const harness = createHarness({
    snapshots: [inventory("v1", {
      sessions: [session("session-1"), session("session-2")],
    })],
  })

  return harness.controller.refreshNow().then(() => {
    harness.controller.applyRowsChanged({
      inventoryVersion: "v2",
      sessions: [session("session-3"), { ...session("session-1"), alias: "updated" }],
      removedSessionIds: ["session-2"],
    })

    assert.deepEqual(harness.availableSessions().map((entry) => [entry.id, entry.alias ?? null]), [
      ["session-1", "updated"],
      ["session-3", null],
    ])
    assert.equal(harness.inventoryCalls(), 1)
    assert.equal(harness.reconcileCount(), 2)
  })
})

test("waiting room refresh still hydrates orphan agents after a row patch set the inventory version", async () => {
  const harness = createHarness({
    snapshots: [inventory("v2", {
      externalProviderSessions: [externalProviderSession("opencode:thread-1")],
      externalProviderSessionsHasMore: true,
      externalProviderSessionsNextCursor: "cursor-2",
    })],
  })

  harness.controller.applyRowsChanged({
    inventoryVersion: "v2",
    sessions: [session("session-1")],
    removedSessionIds: [],
  })
  await harness.controller.refreshNow()

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-1"])
  assert.deepEqual(harness.externalProviderSessions().map((entry) => entry.external_session_id), ["opencode:thread-1"])
  assert.deepEqual(harness.externalProviderSessionsPage(), { hasMore: true, nextCursor: "cursor-2" })
  assert.equal(harness.reconcileCount(), 2)
})

test("waiting room inventory row patch ignores duplicate versions and disconnected kernels", () => {
  const harness = createHarness({ connected: true })

  harness.controller.applyRowsChanged({
    inventoryVersion: "v1",
    sessions: [session("session-1")],
    removedSessionIds: [],
  })
  harness.controller.applyRowsChanged({
    inventoryVersion: "v1",
    sessions: [session("session-2")],
    removedSessionIds: [],
  })

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-1"])
  assert.equal(harness.reconcileCount(), 2)

  const disconnected = createHarness({ connected: false })
  disconnected.controller.applyRowsChanged({
    inventoryVersion: "v1",
    sessions: [session("session-1")],
    removedSessionIds: [],
  })

  assert.deepEqual(disconnected.availableSessions(), [])
  assert.equal(disconnected.reconcileCount(), 0)
})

test("waiting room inventory relay and machine patches apply without fetching", () => {
  const harness = createHarness({ connected: true })

  harness.controller.applyRelayStatusChanged({
    configured: true,
    connected: true,
    relay_token_configured: true,
    daemon_id: "kernel-1",
    machine_id: "machine-1",
    machine_alias: "laptop",
  })
  harness.controller.applyRemoteMachinesChanged([
    {
      machine_id: "machine-2",
      machine_alias: "worker",
      registry_alias: null,
      display_name: "worker",
      trust_status: "approved",
      online: true,
      pending: false,
      kernel_count: 1,
      available_providers: ["opencode"],
    },
  ])

  assert.equal(harness.inventoryCalls(), 0)
  assert.equal(harness.inventoryStatus(), "ready")
  assert.equal(harness.relayStatus()?.daemon_id, "kernel-1")
  assert.deepEqual(harness.remoteMachines().map((machine) => machine.machine_id), ["machine-2"])
  assert.equal(harness.reconcileCount(), 2)

  const disconnected = createHarness({ connected: false })
  disconnected.controller.applyRelayStatusChanged({
    configured: true,
    connected: true,
    relay_token_configured: true,
    daemon_id: "kernel-1",
    machine_id: "machine-1",
  })
  disconnected.controller.applyRemoteMachinesChanged([{
    machine_id: "machine-2",
    display_name: "worker",
    trust_status: "approved",
    online: true,
    pending: false,
    kernel_count: 1,
  }])

  assert.equal(disconnected.relayStatus(), null)
  assert.deepEqual(disconnected.remoteMachines(), [])
  assert.equal(disconnected.reconcileCount(), 0)
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
  let relayStatus: RelayStatusView | null = null
  let remoteMachines: RemoteMachineView[] = []
  let remoteKernels: RemoteKernelView[] = []
  let externalProviderSessions: ExternalProviderSessionRecord[] = []
  let externalProviderSessionsPage = { hasMore: false, nextCursor: null as string | null }
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
    getAvailableSessions: () => availableSessions,
    setAvailableSessions: (sessions) => {
      availableSessions = sessions
    },
    setRelayStatus: (status) => {
      relayStatus = status
    },
    setRemoteMachines: (machines) => {
      remoteMachines = machines
    },
    setRemoteKernels: (kernels) => {
      remoteKernels = kernels
    },
    setTerminals: () => {},
    setSlices: () => {},
    setExternalProviderSessions: (sessions) => {
      externalProviderSessions = sessions
    },
    setExternalProviderSessionsPage: (page) => {
      externalProviderSessionsPage = page
    },
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
    relayStatus: () => relayStatus,
    remoteMachines: () => remoteMachines,
    remoteKernels: () => remoteKernels,
    externalProviderSessions: () => externalProviderSessions,
    externalProviderSessionsPage: () => externalProviderSessionsPage,
    reconcileCount: () => reconcileCount,
    warnings: () => warnings,
  }
}

function externalProviderSession(externalSessionId: string): ExternalProviderSessionRecord {
  const [provider = "opencode", providerSessionId = externalSessionId] = externalSessionId.split(":", 2)
  return {
    external_session_id: externalSessionId,
    provider,
    provider_session_id: providerSessionId,
    title: "External thread",
    last_modified_at_ms: 1,
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

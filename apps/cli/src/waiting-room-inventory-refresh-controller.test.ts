import assert from "node:assert/strict"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type { ExternalProviderSessionRecord, SliceRecord, WaitingRoomPublicSessionSummary } from "./cli-types.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import type { LocalKernelPresence } from "./local-kernel-presence.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
  WaitingRoomInventory,
} from "./waiting-room-inventory-api.js"
import { createWaitingRoomInventoryRefreshController } from "./waiting-room-inventory-refresh-controller.js"
import {
  createWaitingRoomInventoryCache,
  waitingRoomInventoryCacheScopeKey,
} from "./waiting-room-inventory-cache.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { ManagedEnvironmentCatalog } from "@chariox/kernel-client/ipc-managed-environment-requests"

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

test("waiting room inventory refresh applies managed catalog and exact launch target", async () => {
  const managedEnvironmentCatalog: ManagedEnvironmentCatalog = {
    computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
    contextSources: [],
    environments: [],
  }
  const harness = createHarness({
    snapshots: [inventory("v1", {
      managedEnvironmentCatalog,
      launchTarget: { workspaceId: "workspace-1", worktreeId: "worktree-1" },
    })],
  })

  await harness.controller.refreshNow()

  assert.equal(harness.managedEnvironmentCatalog(), managedEnvironmentCatalog)
  assert.deepEqual(harness.launchTarget(), {
    workspaceId: "workspace-1",
    worktreeId: "worktree-1",
  })
})

test("waiting room inventory refresh preserves the last managed catalog on transient omission", async () => {
  const managedEnvironmentCatalog: ManagedEnvironmentCatalog = {
    computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
    contextSources: [],
    environments: [],
  }
  const harness = createHarness({
    snapshots: [
      inventory("v1", { managedEnvironmentCatalog }),
      inventory("v2"),
    ],
  })

  await harness.controller.refreshNow()
  await harness.controller.refreshNow()

  assert.equal(harness.inventoryStatus(), "ready")
  assert.equal(harness.managedEnvironmentCatalog(), managedEnvironmentCatalog)
})

test("waiting room inventory refresh clears retained managed catalog across kernel scope", async () => {
  const managedEnvironmentCatalog: ManagedEnvironmentCatalog = {
    computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
    contextSources: [],
    environments: [],
  }
  const harness = createHarness({
    snapshots: [
      inventory("v1", { kernelId: "kernel-a", managedEnvironmentCatalog }),
      inventory("v2", { kernelId: "kernel-b" }),
    ],
  })

  await harness.controller.refreshNow()
  await harness.controller.refreshNow()

  assert.equal(harness.managedEnvironmentCatalog(), undefined)
})

test("waiting room inventory refresh clears retained managed catalog across Cloud account scope", async () => {
  let scope = "kernel-a:account-a"
  const managedEnvironmentCatalog: ManagedEnvironmentCatalog = {
    computeClasses: [{ computeClass: "agent-small", regions: ["hel1"] }],
    contextSources: [],
    environments: [],
  }
  const harness = createHarness({
    snapshots: [
      inventory("v1", { kernelId: "kernel-a", managedEnvironmentCatalog }),
      inventory("v2", { kernelId: "kernel-a" }),
    ],
    getManagedEnvironmentCatalogScope: () => scope,
  })

  await harness.controller.refreshNow()
  scope = "kernel-a:account-b"
  await harness.controller.refreshNow()

  assert.equal(harness.managedEnvironmentCatalog(), undefined)
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

test("waiting room inventory refresh scopes equal versions to their kernel", async () => {
  const harness = createHarness({
    snapshots: [
      inventory("v1", { kernelId: "kernel-a", sessions: [session("session-a")] }),
      inventory("v1", { kernelId: "kernel-b", sessions: [session("session-b")] }),
    ],
  })

  await harness.controller.refreshNow()
  await harness.controller.refreshNow()

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id).sort(), ["session-a", "session-b"])
})

test("direct target inventory excludes unrelated cached sessions", async () => {
  const cached = inventory("cached", {
    kernelId: "kernel-a",
    sessions: [{ ...session("session-a"), alias: "pr_reviewer" }],
  })
  const harness = createHarness({
    directTargetKernelId: "kernel-b",
    cachedInventories: [cached],
    snapshots: [inventory("fresh", { kernelId: "kernel-b", sessions: [] })],
  })

  assert.deepEqual(harness.availableSessions(), [])
  await harness.controller.refreshNow()
  assert.deepEqual(harness.availableSessions(), [])
})

test("Cloud scope change reloads cache before persisting a fresh direct target", async () => {
  let scope = "local"
  const persisted: Array<{ scope: string; kernelId: string }> = []
  const cached = inventory("cached", {
    kernelId: "kernel-a",
    sessions: [{ ...session("session-a"), alias: "pr_reviewer" }],
  })
  const harness = createHarness({
    directTargetKernelId: "kernel-b",
    cachedInventories: [cached],
    getCacheScopeKey: () => scope,
    loadCachedInventories: () => [],
    persistInventory: (entry) => persisted.push({ scope, kernelId: entry.kernelId }),
    snapshots: [inventory("fresh", { kernelId: "kernel-b", sessions: [] })],
  })

  scope = "cloud-account-b-user-b-realm-b"
  await harness.controller.refreshNow()

  assert.deepEqual(harness.availableSessions(), [])
  assert.deepEqual(persisted, [{ scope: "cloud-account-b-user-b-realm-b", kernelId: "kernel-b" }])
})

test("in-flight inventory from the previous Cloud scope is discarded before retry", async () => {
  const directory = mkdtempSync(join(tmpdir(), "chariox-waiting-room-inflight-scope-"))
  const scopeA = waitingRoomInventoryCacheScopeKey({
    apiUrl: "https://cloud.example",
    accountId: "account-a",
    userId: "user-a",
    realmId: "realm-a",
  })
  const scopeB = waitingRoomInventoryCacheScopeKey({
    apiUrl: "https://cloud.example",
    accountId: "account-b",
    userId: "user-b",
    realmId: "realm-b",
  })
  let scope = scopeA
  try {
    const cache = createWaitingRoomInventoryCache(directory, () => 1_000, {}, () => scope)
    cache.persist(inventory("cached-a", {
      kernelId: "kernel-a",
      sessions: [{ ...session("session-cached-a"), alias: "pr_reviewer" }],
    }))
    const firstFetch = deferred<WaitingRoomInventory>()
    let fetchCount = 0
    const harness = createHarness({
      directTargetKernelId: "kernel-b",
      cachedInventories: cache.load(),
      getCacheScopeKey: () => scope,
      loadCachedInventories: cache.load,
      persistInventory: cache.persist,
      getInventory: () => {
        fetchCount += 1
        return fetchCount === 1
          ? firstFetch.promise
          : Promise.resolve(inventory("fresh-b", { kernelId: "kernel-b", sessions: [] }))
      },
    })

    const refreshing = harness.controller.refreshNow()
    scope = scopeB
    firstFetch.resolve(inventory("stale-a", {
      kernelId: "kernel-a",
      sessions: [{ ...session("session-inflight-a"), alias: "in-flight-a" }],
    }))
    await refreshing

    assert.equal(fetchCount, 2)
    assert.deepEqual(harness.availableSessions(), [])
    assert.deepEqual(cache.load().map((entry) => entry.kernelId), ["kernel-b"])
    scope = scopeA
    assert.deepEqual(
      cache.load().flatMap((entry) => entry.sessions.map((entry) => entry.alias)),
      ["pr_reviewer"],
    )
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test("repeated in-flight scope changes settle the refresh as an error", async () => {
  let scope = "scope-a"
  const firstFetch = deferred<WaitingRoomInventory>()
  const secondFetch = deferred<WaitingRoomInventory>()
  const secondStarted = deferred<void>()
  let fetchCount = 0
  const harness = createHarness({
    getCacheScopeKey: () => scope,
    loadCachedInventories: () => [],
    getInventory: () => {
      fetchCount += 1
      if (fetchCount === 1) return firstFetch.promise
      secondStarted.resolve()
      return secondFetch.promise
    },
  })

  const refreshing = harness.controller.refreshNow()
  scope = "scope-b"
  firstFetch.resolve(inventory("scope-a", { kernelId: "kernel-a" }))
  await secondStarted.promise
  scope = "scope-c"
  secondFetch.resolve(inventory("scope-b", { kernelId: "kernel-b" }))
  await refreshing

  assert.equal(harness.inventoryStatus(), "error")
  assert.deepEqual(harness.availableSessions(), [])
  assert.equal(harness.warnings().at(-1)?.message, "waiting room inventory scope changed repeatedly during refresh")
})

test("direct target projection follows a kernel pivot and rollback", () => {
  let directTargetKernelId = "kernel-a"
  const harness = createHarness({
    cachedInventories: [
      inventory("cached-a", { kernelId: "kernel-a", sessions: [session("session-a")] }),
      inventory("cached-b", { kernelId: "kernel-b", sessions: [session("session-b")] }),
    ],
    getDirectTargetKernelId: () => directTargetKernelId,
  })

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-a"])
  directTargetKernelId = "kernel-b"
  harness.controller.invalidate()
  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-b"])
  directTargetKernelId = "kernel-a"
  harness.controller.invalidate()
  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-a"])
})

test("old client rejection during a target pivot retries the new target", async () => {
  let directTargetKernelId = "kernel-a"
  const firstFetch = deferred<WaitingRoomInventory>()
  let fetchCount = 0
  const harness = createHarness({
    getDirectTargetKernelId: () => directTargetKernelId,
    getInventory: () => {
      fetchCount += 1
      return fetchCount === 1
        ? firstFetch.promise
        : Promise.resolve(inventory("fresh-b", {
            kernelId: "kernel-b",
            sessions: [session("session-b")],
          }))
    },
  })

  const refreshing = harness.controller.refreshNow()
  directTargetKernelId = "kernel-b"
  harness.controller.invalidate()
  firstFetch.reject(new Error("old client closed during pivot"))
  await refreshing

  assert.equal(fetchCount, 2)
  assert.equal(harness.inventoryStatus(), "ready")
  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-b"])
  assert.deepEqual(harness.warnings(), [])
})

test("waiting room inventory refresh makes fresh local sibling kernels visible", async () => {
  const harness = createHarness({
    snapshots: [inventory("v1", { kernelId: "kernel-a", machineId: "machine-a" })],
    localKernelPresences: [
      { kernelId: "kernel-a", machineId: "machine-a", host: "127.0.0.1", port: 43_121, heartbeatAtMs: 1 },
      { kernelId: "kernel-b", kernelAlias: "Experiments", machineId: "machine-a", machineAlias: "Laptop", host: "127.0.0.1", port: 43_122, heartbeatAtMs: 1 },
    ],
  })

  await harness.controller.refreshNow()

  assert.equal(harness.remoteMachines()[0]?.kernel_count, 2)
  assert.deepEqual(harness.remoteKernels().map((entry) => entry.kernel_id), ["kernel-b"])
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
    structuralVersion: "structure-v1",
    activityRevision: "activity-v1",
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
      structuralVersion: "structure-v2",
      activityRevision: "activity-v2",
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

test("waiting room inventory row patch merges project changes and removals", async () => {
  const harness = createHarness({
    snapshots: [inventory("v1", {
      projects: [project("project-1", "Frontend"), project("project-2", "Docs")],
    })],
  })

  await harness.controller.refreshNow()
  harness.controller.applyRowsChanged({
    inventoryVersion: "v2",
    structuralVersion: "structure-v2",
    activityRevision: "activity-v2",
    sessions: [],
    removedSessionIds: [],
    projects: [{ ...project("project-1", "Web"), updated_at_ms: 5 }],
    removedProjectIds: ["project-2"],
  })

  assert.deepEqual(harness.projects().map(({ id, name }) => [id, name]), [["project-1", "Web"]])
})

test("waiting room refresh still hydrates unattached agents after a row patch set the inventory version", async () => {
  const harness = createHarness({
    snapshots: [inventory("v2", {
      externalProviderSessions: [
        externalProviderSession("opencode:thread-1", { last_modified_at_ms: 100 }),
        externalProviderSession("codex:thread-2", { last_modified_at_ms: 200 }),
      ],
      externalProviderSessionsHasMore: true,
      externalProviderSessionsNextCursor: "cursor-2",
    })],
  })

  harness.controller.applyRowsChanged({
    inventoryVersion: "v2",
    structuralVersion: "structure-v2",
    activityRevision: "activity-v2",
    sessions: [session("session-1")],
    removedSessionIds: [],
  })
  await harness.controller.refreshNow()

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-1"])
  assert.deepEqual(harness.externalProviderSessions().map((entry) => entry.external_session_id), [
    "codex:thread-2",
    "opencode:thread-1",
  ])
  assert.deepEqual(harness.externalProviderSessionsPage(), { hasMore: true, nextCursor: "cursor-2" })
  assert.equal(harness.reconcileCount(), 2)
})

test("waiting room inventory row patch ignores duplicate versions and disconnected kernels", () => {
  const harness = createHarness({ connected: true })

  harness.controller.applyRowsChanged({
    inventoryVersion: "v1",
    structuralVersion: "structure-v1",
    activityRevision: "activity-v1",
    sessions: [session("session-1")],
    removedSessionIds: [],
  })
  harness.controller.applyRowsChanged({
    inventoryVersion: "v1",
    structuralVersion: "structure-v1",
    activityRevision: "activity-v1",
    sessions: [session("session-2")],
    removedSessionIds: [],
  })

  assert.deepEqual(harness.availableSessions().map((entry) => entry.id), ["session-1"])
  assert.equal(harness.reconcileCount(), 2)

  const disconnected = createHarness({ connected: false })
  disconnected.controller.applyRowsChanged({
    inventoryVersion: "v1",
    structuralVersion: "structure-v1",
    activityRevision: "activity-v1",
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
  localKernelPresences?: readonly LocalKernelPresence[]
  cachedInventories?: readonly WaitingRoomInventory[]
  directTargetKernelId?: string | null
  getDirectTargetKernelId?: () => string | null | undefined
  getCacheScopeKey?: () => string
  loadCachedInventories?: () => readonly WaitingRoomInventory[]
  persistInventory?: (inventory: WaitingRoomInventory) => void
  getManagedEnvironmentCatalogScope?: (inventory: WaitingRoomInventory) => string
} = {}) {
  const catalog = fallbackProviderCatalog()
  const hiddenKernelIds = options.hiddenKernelIds ?? new Set<string>()
  let inventoryStatus: "loading" | "ready" | "error" = "loading"
  let waitingRoomState = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "medium")
  let availableSessions: SessionListEntry[] = []
  let relayStatus: RelayStatusView | null = null
  let remoteMachines: RemoteMachineView[] = []
  let remoteKernels: RemoteKernelView[] = []
  let projects: WaitingRoomProjectSummary[] = []
  let externalProviderSessions: ExternalProviderSessionRecord[] = []
  let externalProviderSessionsPage = { hasMore: false, nextCursor: null as string | null }
  let managedEnvironmentCatalog: ManagedEnvironmentCatalog | undefined
  let launchTarget: WaitingRoomInventory["launchTarget"]
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
    getProjects: () => projects,
    setProjects: (nextProjects) => {
      projects = nextProjects
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
    setManagedEnvironmentCatalog: (catalog) => {
      managedEnvironmentCatalog = catalog
    },
    ...(options.getManagedEnvironmentCatalogScope
      ? { getManagedEnvironmentCatalogScope: options.getManagedEnvironmentCatalogScope }
      : {}),
    setLaunchTarget: (target) => {
      launchTarget = target
    },
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
    getLocalKernelPresences: () => options.localKernelPresences ?? [],
    ...(options.cachedInventories ? { cachedInventories: options.cachedInventories } : {}),
    ...(options.directTargetKernelId !== undefined
      ? { directTargetKernelId: options.directTargetKernelId }
      : {}),
    ...(options.getDirectTargetKernelId
      ? { getDirectTargetKernelId: options.getDirectTargetKernelId }
      : {}),
    ...(options.getCacheScopeKey ? { getCacheScopeKey: options.getCacheScopeKey } : {}),
    ...(options.loadCachedInventories ? { loadCachedInventories: options.loadCachedInventories } : {}),
    ...(options.persistInventory ? { persistInventory: options.persistInventory } : {}),
  })

  return {
    controller,
    inventoryCalls: () => inventoryCalls,
    inventoryStatus: () => inventoryStatus,
    availableSessions: () => availableSessions,
    relayStatus: () => relayStatus,
    remoteMachines: () => remoteMachines,
    remoteKernels: () => remoteKernels,
    projects: () => projects,
    externalProviderSessions: () => externalProviderSessions,
    externalProviderSessionsPage: () => externalProviderSessionsPage,
    managedEnvironmentCatalog: () => managedEnvironmentCatalog,
    launchTarget: () => launchTarget,
    reconcileCount: () => reconcileCount,
    warnings: () => warnings,
  }
}

function externalProviderSession(
  externalSessionId: string,
  overrides: Partial<ExternalProviderSessionRecord> = {},
): ExternalProviderSessionRecord {
  const [provider = "opencode", providerSessionId = externalSessionId] = externalSessionId.split(":", 2)
  return {
    external_session_id: externalSessionId,
    provider,
    provider_session_id: providerSessionId,
    title: "External thread",
    last_modified_at_ms: 1,
    ...overrides,
  }
}

function inventory(
  inventoryVersion: string,
  overrides: Partial<WaitingRoomInventory> = {},
): WaitingRoomInventory {
  return {
    schemaVersion: 11,
    inventoryVersion,
    structuralVersion: `structure-${inventoryVersion}`,
    activityRevision: `activity-${inventoryVersion}`,
    kernelId: "kernel-1",
    machineId: "machine-1",
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
    project_id: "project-default",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: "Created",
    created_at_ms: 1,
    connected_cli_count: 0,
  }
}

function project(id: string, name: string): WaitingRoomProjectSummary {
  return {
    id,
    owner_user_id: "owner",
    workspace_id: "/workspace",
    name,
    kind: "named",
    status: "active",
    created_at_ms: 1,
    updated_at_ms: 2,
    session_count: 0,
    joined_collaborator_count: 0,
    pending_collaboration_invite_count: 0,
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
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

import { waitingRoomSessionRecencyMs } from "@chariox/kernel-client/waiting-room-activity"
import { createWaitingRoomRetainedRows } from "./waiting-room-retained-rows.js"
import {
  externalProviderSessionPageSessions,
  externalProviderSessionPageState,
} from "@chariox/kernel-client/external-provider-sessions"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import type { ExternalProviderSessionRecord, ProviderAccountProfile, SliceRecord, WaitingRoomPublicSessionSummary } from "./cli-types.js"
import type { LocalKernelPresence } from "./local-kernel-presence.js"
import { waitingRoomRemoteKernelCanDelete } from "./waiting-room-remote-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
  WaitingRoomInventory,
} from "./waiting-room-inventory-api.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"
import type { ManagedEnvironmentCatalog } from "@chariox/kernel-client/ipc-managed-environment-requests"
import { LocalIpcError, type LocalIpcClient } from "./ipc.js"

type WaitingRoomInventoryStatus = "loading" | "ready" | "error"
const maximumTrackedKernelInventories = 64

type WaitingRoomRowsChangedPatch = {
  inventoryVersion: string
  structuralVersion: string
  activityRevision: string
  sessions: WaitingRoomPublicSessionSummary[]
  removedSessionIds: string[]
  sessionsTotal?: number
  sessionsNextCursor?: string | null
  projects?: WaitingRoomProjectSummary[]
  removedProjectIds?: string[]
}

type WaitingRoomInventoryRefreshControllerOptions = {
  nowMs?: () => number
  isKernelConnected: () => boolean
  getInventoryStatus: () => WaitingRoomInventoryStatus
  setInventoryStatus: (status: WaitingRoomInventoryStatus) => void
  getWaitingRoomState: () => WaitingRoomState
  getInventory: () => Promise<WaitingRoomInventory>
  getSessionPage?: (cursor: string | null, inventoryVersion: string) => Promise<{ sessions: WaitingRoomPublicSessionSummary[]; next_cursor: string | null }>
  isKernelHidden: (kernelId: string) => boolean
  getAvailableSessions: () => SessionListEntry[]
  setAvailableSessions: (sessions: SessionListEntry[]) => void
  getProjects?: () => WaitingRoomProjectSummary[]
  setProjects?: (projects: WaitingRoomProjectSummary[]) => void
  setRelayStatus: (status: RelayStatusView) => void
  setRemoteMachines: (machines: RemoteMachineView[]) => void
  setRemoteKernels: (kernels: RemoteKernelView[]) => void
  setTerminals: (terminals: TerminalView[]) => void
  setSlices: (slices: SliceRecord[]) => void
  setProviderAccounts?: (profiles: ProviderAccountProfile[]) => void
  setManagedEnvironmentCatalog?: (catalog: ManagedEnvironmentCatalog | undefined) => void
  getManagedEnvironmentCatalogScope?: (inventory: WaitingRoomInventory) => string
  shouldApplyWorkspaceInventory?: (inventory: WaitingRoomInventory) => boolean
  applyWorkspaceInventory?: (inventory: WaitingRoomInventory, client?: LocalIpcClient) => void | Promise<void>
  setLaunchTarget?: (target: WaitingRoomInventory["launchTarget"]) => void
  setExternalProviderSessions?: (sessions: ExternalProviderSessionRecord[]) => void
  setExternalProviderSessionsPage?: (page: { hasMore: boolean; nextCursor: string | null }) => void
  reconcileWaitingRoom: (state: WaitingRoomState) => void
  warn?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
  cachedInventories?: readonly WaitingRoomInventory[]
  getCacheScopeKey?: () => string
  loadCachedInventories?: () => readonly WaitingRoomInventory[]
  directTargetKernelId?: string | null
  getDirectTargetKernelId?: () => string | null | undefined
  persistInventory?: (inventory: WaitingRoomInventory) => void
  getLocalKernelPresences?: () => readonly LocalKernelPresence[]
}

export type WaitingRoomInventoryRefreshController = {
  applyWorkspacePreview(inventory: WaitingRoomInventory, client: LocalIpcClient): Promise<void>
  applyTransportClosed(): void
  applyRowsChanged(patch: WaitingRoomRowsChangedPatch): void
  applyRelayStatusChanged(status: RelayStatusView): void
  applyRemoteMachinesChanged(machines: RemoteMachineView[]): void
  loadSessionHistory(): Promise<void>
  refreshNow(): Promise<void>
  refresh(): Promise<void>
  invalidate(): void
}

export function createWaitingRoomInventoryRefreshController(
  options: WaitingRoomInventoryRefreshControllerOptions,
): WaitingRoomInventoryRefreshController {
  const formatError = options.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))
  let inventoryVersion: string | null = null
  let activeKernelId: string | null = null
  let managedEnvironmentCatalogScope: string | null = null
  let inventoryInvalidated = false
  let transportGeneration = 0
  let rowsBaselinePending = false
  let cacheScopeKey = options.getCacheScopeKey?.() ?? null
  let directTargetKernelId = currentDirectTargetKernelId(options)
  const inventoriesByKernel = new Map(
    (options.cachedInventories ?? []).map((inventory) => [inventory.kernelId, inventory]),
  )
  let pendingRefresh: Promise<void> | null = null
  const nowMs = options.nowMs ?? Date.now
  const lastPresenceByKernel = new Map([...inventoriesByKernel.keys()].map(id => [id, nowMs()]))

  const retainedMachines = createWaitingRoomRetainedRows<RemoteMachineView>(row => row.machine_id, options.nowMs ?? Date.now)
  const retainedKernels = createWaitingRoomRetainedRows<RemoteKernelView>(row => row.kernel_id, options.nowMs ?? Date.now)
  const setProjects = (projects: WaitingRoomProjectSummary[]) => {
    const state = options.getWaitingRoomState()
    const focused = state.focus === "project-entry"
      ? waitingRoomProjectsForNavigation(options.getProjects?.(), Boolean(state.showArchivedProjects))[state.projectIndex ?? 0]?.id
      : undefined
    options.setProjects?.(projects)
    if (!focused) return
    const index = waitingRoomProjectsForNavigation(projects, Boolean(state.showArchivedProjects)).findIndex(project => project.id === focused)
    if (index >= 0 && index !== (state.projectIndex ?? 0)) options.reconcileWaitingRoom({ ...state, projectIndex: index })
  }

  const restoreCachedInventory = () => {
    options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
    const snapshots = [...inventoriesByKernel.values()].filter(snapshot => !directTargetKernelId || snapshot.kernelId === directTargetKernelId)
    const machines = [...new Map(snapshots.flatMap(snapshot => snapshot.remoteMachines).map(row => [row.machine_id, row])).values()]
    const kernels = [...new Map(snapshots.flatMap(snapshot => snapshot.remoteKernels).map(row => [row.kernel_id, row])).values()]
    options.setRemoteMachines(retainedMachines.restore(machines).map(row => ({ ...row, online: false })))
    options.setRemoteKernels(retainedKernels.restore(kernels).map(row => ({ ...row, accepting_remote_leases: false })))
    const workspaceSnapshots = snapshots.filter(snapshot => options.shouldApplyWorkspaceInventory?.(snapshot) !== false)
    if (workspaceSnapshots.length || !snapshots.length) {
      setProjects([...new Map(workspaceSnapshots.flatMap(snapshot => snapshot.projects ?? []).map(row => [row.id, row])).values()])
    }
  }
  if ((options.cachedInventories?.length ?? 0) > 0) restoreCachedInventory()

  const rememberInventory = (inventory: WaitingRoomInventory) => {
    const previous = inventoriesByKernel.get(inventory.kernelId)
    if (previous?.structuralVersion === inventory.structuralVersion && previous.sessions.length > inventory.sessions.length) {
      inventory = { ...inventory, sessions: [...new Map([...previous.sessions, ...inventory.sessions].map(row => [row.id, row])).values()] }
    }
    inventoriesByKernel.delete(inventory.kernelId)
    inventoriesByKernel.set(inventory.kernelId, inventory)
    while (inventoriesByKernel.size > maximumTrackedKernelInventories) {
      let oldestInactiveKernelId: string | undefined
      for (const kernelId of inventoriesByKernel.keys()) {
        if (kernelId !== activeKernelId) {
          oldestInactiveKernelId = kernelId
          break
        }
      }
      if (!oldestInactiveKernelId) {
        break
      }
      inventoriesByKernel.delete(oldestInactiveKernelId)
    }
  }

  const retainDisconnectedRows = () => {
    options.setRemoteMachines(retainedMachines.reconcile([], { authoritative: false }).map(row => ({ ...row, online: false })))
    options.setRemoteKernels(retainedKernels.reconcile([], { authoritative: false }).map(row => ({ ...row, accepting_remote_leases: false })))
  }

  const refreshNow = async () => {
    if (!options.isKernelConnected()) {
      retainDisconnectedRows()
      return
    }
    if (options.getInventoryStatus() !== "ready") {
      options.setInventoryStatus("loading")
    }
    for (let attempt = 0; attempt < 2; attempt += 1) {
      refreshProjectionScope()
      const requestedTransportGeneration = transportGeneration
      const requestedCacheScopeKey = cacheScopeKey
      const requestedTargetKernelId = directTargetKernelId
      let snapshot: WaitingRoomInventory
      try {
        snapshot = await options.getInventory()
      } catch (error) {
        if (requestedTransportGeneration !== transportGeneration) return
        refreshProjectionScope()
        if (
          requestedCacheScopeKey !== cacheScopeKey
          || requestedTargetKernelId !== directTargetKernelId
        ) {
          options.setInventoryStatus("loading")
          continue
        }
        options.warn?.("waiting room inventory refresh failed", { error: formatError(error) })
        options.setInventoryStatus("error")
        retainDisconnectedRows()
        options.reconcileWaitingRoom(options.getWaitingRoomState())
        return
      }
      if (requestedTransportGeneration !== transportGeneration) return
      refreshProjectionScope()
      if (
        requestedCacheScopeKey !== cacheScopeKey
        || requestedTargetKernelId !== directTargetKernelId
      ) {
        options.setInventoryStatus("loading")
        continue
      }
      options.setInventoryStatus("ready")
      const present = new Set([snapshot.kernelId, ...snapshot.remoteKernels.map(row => row.kernel_id), ...(options.getLocalKernelPresences?.() ?? []).map(row => row.kernelId)])
      let pruned = false
      for (const id of inventoriesByKernel.keys()) {
        if (present.has(id)) lastPresenceByKernel.set(id, nowMs())
        else if (nowMs() - (lastPresenceByKernel.get(id) ?? nowMs()) >= 30_000) {
          inventoriesByKernel.delete(id)
          lastPresenceByKernel.delete(id)
          pruned = true
        }
      }
      if (pruned) options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
      lastPresenceByKernel.set(snapshot.kernelId, nowMs())
      const previousActiveKernelId = activeKernelId
      const previousInventoryVersion = inventoryVersion
      activeKernelId = snapshot.kernelId
      inventoryVersion = inventoryInvalidated
        ? null
        : inventoriesByKernel.get(snapshot.kernelId)?.inventoryVersion
          ?? (previousActiveKernelId === null ? previousInventoryVersion : null)
      inventoryInvalidated = false
      if (snapshot.inventoryVersion === inventoryVersion) {
        applySupplementalInventory(snapshot)
        options.reconcileWaitingRoom(options.getWaitingRoomState())
        return
      }

      inventoryVersion = snapshot.inventoryVersion
      rememberInventory(snapshot)
      options.persistInventory?.(snapshot)
      options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
      applySupplementalInventory(snapshot)
      options.reconcileWaitingRoom(options.getWaitingRoomState())
      return
    }
    options.setInventoryStatus("error")
    options.warn?.("waiting room inventory scope changed repeatedly during refresh", {
      cache_scope_changed: true,
    })
  }

  const applySupplementalInventory = (snapshot: WaitingRoomInventory) => {
    const localPresence = mergeLocalKernelPresence(
      snapshot.remoteMachines,
      snapshot.remoteKernels,
      options.getLocalKernelPresences?.() ?? [],
      snapshot.kernelId,
      inventoriesByKernel,
    )
    options.setRelayStatus(snapshot.relayStatus)
    options.setRemoteMachines(retainedMachines.reconcile(localPresence.machines).map(row => row.displayFreshness ? { ...row, online: false } : row))
    options.setRemoteKernels(retainedKernels.reconcile(localPresence.kernels).map(row => row.displayFreshness ? { ...row, accepting_remote_leases: false } : row).filter((kernel) => (
      !options.isKernelHidden(kernel.kernel_id)
      || Boolean(kernel.displayFreshness)
      || !waitingRoomRemoteKernelCanDelete(kernel)
    )))
    options.setTerminals(snapshot.terminals)
    options.setSlices(snapshot.slices)
    options.setProviderAccounts?.(snapshot.providerAccounts ?? [])
    const nextManagedEnvironmentCatalogScope = options.getManagedEnvironmentCatalogScope?.(snapshot)
      ?? snapshot.kernelId
    if (snapshot.managedEnvironmentCatalog !== undefined) {
      managedEnvironmentCatalogScope = nextManagedEnvironmentCatalogScope
      options.setManagedEnvironmentCatalog?.(snapshot.managedEnvironmentCatalog)
    } else if (managedEnvironmentCatalogScope !== nextManagedEnvironmentCatalogScope) {
      managedEnvironmentCatalogScope = nextManagedEnvironmentCatalogScope
      options.setManagedEnvironmentCatalog?.(undefined)
    }
    if (options.shouldApplyWorkspaceInventory?.(snapshot) !== false) {
      options.applyWorkspaceInventory?.(snapshot)
      options.setLaunchTarget?.(snapshot.launchTarget)
      setProjects(snapshot.projects ?? [])
    }
    const externalProviderSessionsPage = {
      ...(snapshot.externalProviderSessions !== undefined ? { externalProviderSessions: snapshot.externalProviderSessions } : {}),
      ...(snapshot.externalProviderSessionsHasMore !== undefined
        ? { externalProviderSessionsHasMore: snapshot.externalProviderSessionsHasMore }
        : {}),
      ...(snapshot.externalProviderSessionsNextCursor !== undefined
        ? { externalProviderSessionsNextCursor: snapshot.externalProviderSessionsNextCursor }
        : {}),
    }
    options.setExternalProviderSessions?.(externalProviderSessionPageSessions(externalProviderSessionsPage))
    options.setExternalProviderSessionsPage?.(externalProviderSessionPageState(externalProviderSessionsPage))
  }

  const refreshProjectionScope = () => {
    const nextScopeKey = options.getCacheScopeKey?.() ?? null
    const nextDirectTargetKernelId = currentDirectTargetKernelId(options)
    const cacheScopeChanged = nextScopeKey !== cacheScopeKey
    const directTargetChanged = nextDirectTargetKernelId !== directTargetKernelId
    if (!cacheScopeChanged && !directTargetChanged) {
      return
    }
    if (cacheScopeChanged) {
      cacheScopeKey = nextScopeKey
      inventoriesByKernel.clear()
      lastPresenceByKernel.clear()
      for (const inventory of options.loadCachedInventories?.() ?? []) {
        inventoriesByKernel.set(inventory.kernelId, inventory)
        lastPresenceByKernel.set(inventory.kernelId, nowMs())
      }
    }
    directTargetKernelId = nextDirectTargetKernelId
    activeKernelId = null
    inventoryVersion = null
    inventoryInvalidated = true
    restoreCachedInventory()
  }

  return {
    async loadSessionHistory() {
      await refreshNow()
      if (!options.isKernelConnected() || !activeKernelId) throw new Error("Reconnect the kernel before opening older sessions.")
      const kernelId = activeKernelId
      const generation = transportGeneration
      const initial = inventoriesByKernel.get(kernelId)
      if (!initial || (initial.sessionsTotal ?? initial.sessions.length) <= initial.sessions.length) return
      if (!options.getSessionPage) throw new Error("This client cannot load older sessions. Update Chariox and reopen the session list.")
      for (let attempt = 0; attempt < 3; attempt++) {
        const baseline = inventoriesByKernel.get(kernelId)!
        let cursor: string | null = null
        const cursors = new Set<string>()
        const sessions = new Map<string, WaitingRoomPublicSessionSummary>()
        try {
          do {
            const page = await options.getSessionPage(cursor, baseline.inventoryVersion)
            refreshProjectionScope()
            const inventory = inventoriesByKernel.get(kernelId)
            if (generation !== transportGeneration || kernelId !== activeKernelId) {
              throw new Error("The kernel connection changed. Reopen the session list.")
            }
            if (inventory?.inventoryVersion !== baseline.inventoryVersion) {
              throw new LocalIpcError("list sessions", "Session activity changed during listing", "waiting_room_inventory_changed")
            }
            for (const row of page.sessions) sessions.set(row.id, { ...row, kernel_id: inventory.kernelId, kernel_alias: inventory.kernelAlias ?? null, machine_id: inventory.machineId, machine_alias: inventory.machineAlias ?? null })
            cursor = page.next_cursor
            if (cursor !== null && cursors.has(cursor)) throw new Error("The kernel repeated a session page. Reopen the session list.")
            if (cursor !== null) cursors.add(cursor)
          } while (cursor !== null)
          if (baseline.sessionsTotal !== undefined && sessions.size !== baseline.sessionsTotal) throw new Error("The kernel returned an incomplete session list. Reopen the session list.")
          rememberInventory({ ...baseline, sessions: [...sessions.values()] })
          options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
          options.reconcileWaitingRoom(options.getWaitingRoomState())
          return
        } catch (error) {
          if (!(error instanceof LocalIpcError) || error.code !== "waiting_room_inventory_changed" || attempt === 2) throw error
          await refreshNow()
        }
      }
    },
    applyTransportClosed() {
      transportGeneration += 1
      rowsBaselinePending = true
      inventoryVersion = null
      inventoryInvalidated = true
      options.setInventoryStatus("loading")
      retainDisconnectedRows()
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    async applyWorkspacePreview(inventory, client) {
      rememberInventory(inventory)
      options.persistInventory?.(inventory)
      options.setAvailableSessions(visibleSessions(inventoriesByKernel, currentDirectTargetKernelId(options)))
      options.setLaunchTarget?.(inventory.launchTarget)
      setProjects(inventory.projects ?? [])
      await options.applyWorkspaceInventory?.(inventory, client)
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRowsChanged(patch) {
      if (!options.isKernelConnected()) {
        return
      }
      refreshProjectionScope()
      const activeInventory = activeKernelId ? inventoriesByKernel.get(activeKernelId) : undefined
      // A row patch cannot establish the initial snapshot. Publishing its
      // sessions would also mistake them for cached display before cold load.
      if (!activeInventory) return
      if (!rowsBaselinePending && patch.inventoryVersion === inventoryVersion) {
        options.reconcileWaitingRoom(options.getWaitingRoomState())
        return
      }
      const currentInventoryVersion = rowsBaselinePending ? null : inventoryVersion
      rowsBaselinePending = false
      inventoryVersion = patch.inventoryVersion
      options.setInventoryStatus("ready")
      const recentIds = new Set(activeInventory.recentSessionIds ?? activeInventory.sessions.map(row => row.id))
      const knownIds = new Set(activeInventory.sessions.map(row => row.id))
      const beforeTotal = activeInventory.sessionsTotal ?? activeInventory.sessions.length
      const afterTotal = patch.sessionsTotal ?? beforeTotal
      // The kernel names every row that left its bounded recent window, deleted or merely displaced by a
      // newer Room. The totals tell them apart: a Room that is gone for good makes the total fall short of
      // the known Rooms plus the new ones. With no such shortfall every named row still exists.
      const newRowCount = patch.sessions.filter(row => !knownIds.has(row.id)).length
      const historyInvalid = beforeTotal + newRowCount > afterTotal
      const previousSessions = historyInvalid
        ? activeInventory.sessions.filter(row => recentIds.has(row.id))
        : activeInventory.sessions
      const removedSessionIds = historyInvalid ? patch.removedSessionIds : []
      const sessions = mergeWaitingRoomSessionRows(
        currentInventoryVersion === null ? [] : previousSessions,
        patch.sessions.map((session) => ({
          ...session,
          kernel_id: activeInventory.kernelId,
          kernel_alias: activeInventory.kernelAlias ?? null,
          machine_id: activeInventory.machineId,
          machine_alias: activeInventory.machineAlias ?? null,
        })),
        removedSessionIds,
      )
      const nextRecentIds = new Set([...recentIds].filter(id => !patch.removedSessionIds.includes(id)).concat(patch.sessions.map(row => row.id)))
      const nextInventory = {
        ...activeInventory,
        inventoryVersion: patch.inventoryVersion,
        structuralVersion: patch.structuralVersion,
        activityRevision: patch.activityRevision,
        recentSessionIds: sessions.filter(row => nextRecentIds.has(row.id)).map(row => row.id),
        sessionsTotal: afterTotal,
        ...(patch.sessionsNextCursor !== undefined ? { sessionsNextCursor: patch.sessionsNextCursor } : {}),
        sessions,
        projects: patch.projects
          ? mergeWaitingRoomProjectRows(
              currentInventoryVersion === null ? [] : activeInventory.projects ?? [],
              patch.projects,
              patch.removedProjectIds ?? [],
            )
          : activeInventory.projects ?? [],
      }
      rememberInventory(nextInventory)
      options.persistInventory?.(nextInventory)
      options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
      if (options.shouldApplyWorkspaceInventory?.(nextInventory) !== false) {
        setProjects(nextInventory.projects)
      }
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRelayStatusChanged(status) {
      if (!options.isKernelConnected()) {
        return
      }
      refreshProjectionScope()
      options.setRelayStatus(status)
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRemoteMachinesChanged(machines) {
      if (!options.isKernelConnected()) {
        return
      }
      refreshProjectionScope()
      const localPresence = mergeLocalKernelPresence(
        machines,
        [],
        options.getLocalKernelPresences?.() ?? [],
        activeKernelId,
        inventoriesByKernel,
      )
      options.setRemoteMachines(retainedMachines.reconcile(localPresence.machines).map(row => row.displayFreshness ? { ...row, online: false } : row))
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    refreshNow,
    refresh() {
      if (pendingRefresh) {
        return pendingRefresh
      }
      pendingRefresh = refreshNow().finally(() => {
        pendingRefresh = null
      })
      return pendingRefresh
    },
    invalidate() {
      refreshProjectionScope()
      inventoryVersion = null
      inventoryInvalidated = true
    },
  }
}

function mergeWaitingRoomProjectRows(
  current: readonly WaitingRoomProjectSummary[],
  changed: readonly WaitingRoomProjectSummary[],
  removedIds: readonly string[],
): WaitingRoomProjectSummary[] {
  const removed = new Set(removedIds)
  const byId = new Map(current.filter((project) => !removed.has(project.id)).map((project) => [project.id, project]))
  for (const project of changed) {
    if (!removed.has(project.id)) byId.set(project.id, project)
  }
  return Array.from(byId.values()).sort((left, right) => (
    (right.last_session_activity_at_ms ?? right.updated_at_ms)
    - (left.last_session_activity_at_ms ?? left.updated_at_ms)
  ))
}

function mergedCachedSessions(inventories: Iterable<WaitingRoomInventory>): SessionListEntry[] {
  return Array.from(inventories)
    .flatMap((inventory) => inventory.sessions)
    .sort((left, right) => (
      waitingRoomSessionRecencyMs(right) - waitingRoomSessionRecencyMs(left)
      || (left.created_at_ms ?? 0) - (right.created_at_ms ?? 0) || left.id.localeCompare(right.id)
    ))
}

function visibleSessions(
  inventoriesByKernel: ReadonlyMap<string, WaitingRoomInventory>,
  directTargetKernelId: string | null | undefined,
): SessionListEntry[] {
  const targetKernelId = directTargetKernelId?.trim()
  if (targetKernelId) {
    const target = inventoriesByKernel.get(targetKernelId)
    return target ? mergedCachedSessions([target]) : []
  }
  return mergedCachedSessions(inventoriesByKernel.values())
}

function currentDirectTargetKernelId(
  options: Pick<
    WaitingRoomInventoryRefreshControllerOptions,
    "directTargetKernelId" | "getDirectTargetKernelId"
  >,
): string | null {
  return (options.getDirectTargetKernelId?.() ?? options.directTargetKernelId)?.trim() || null
}

function mergeLocalKernelPresence(
  machines: readonly RemoteMachineView[],
  kernels: readonly RemoteKernelView[],
  presences: readonly LocalKernelPresence[],
  currentKernelId: string | null,
  inventoriesByKernel: ReadonlyMap<string, WaitingRoomInventory>,
): { machines: RemoteMachineView[]; kernels: RemoteKernelView[] } {
  const activePresences = presences.filter((presence) => presence.kernelId !== currentKernelId)
  const kernelsById = new Map(kernels.map((kernel) => [kernel.kernel_id, kernel]))
  for (const presence of activePresences) {
    if (kernelsById.has(presence.kernelId)) {
      continue
    }
    kernelsById.set(presence.kernelId, {
      kernel_id: presence.kernelId,
      kernel_alias: presence.kernelAlias ?? null,
      machine_id: presence.machineId,
      machine_alias: presence.machineAlias ?? null,
      capabilities: ["kernel_ws", "local_presence"],
      local_session_count: inventoriesByKernel.get(presence.kernelId)?.sessions.length ?? 0,
    })
  }

  const localKernelCountByMachine = new Map<string, number>()
  for (const presence of presences) {
    localKernelCountByMachine.set(
      presence.machineId,
      (localKernelCountByMachine.get(presence.machineId) ?? 0) + 1,
    )
  }
  const machinesById = new Map(machines.map((machine) => [machine.machine_id, machine]))
  for (const presence of activePresences) {
    const existing = machinesById.get(presence.machineId)
    const localKernelCount = localKernelCountByMachine.get(presence.machineId) ?? 1
    machinesById.set(presence.machineId, existing
      ? { ...existing, online: true, kernel_count: Math.max(existing.kernel_count, localKernelCount) }
      : {
          machine_id: presence.machineId,
          machine_alias: presence.machineAlias ?? null,
          display_name: presence.machineAlias ?? presence.machineId,
          trust_status: "approved",
          online: true,
          pending: false,
          kernel_count: localKernelCount,
        })
  }
  return { machines: [...machinesById.values()], kernels: [...kernelsById.values()] }
}

function mergeWaitingRoomSessionRows<T extends SessionListEntry>(
  current: T[],
  changed: T[],
  removedIds: string[],
): T[] {
  const removed = new Set(removedIds)
  const changedById = new Map(changed.map((session) => [session.id, session]))
  const merged = current
    .filter((session) => !removed.has(session.id))
    .map((session) => changedById.get(session.id) ?? session)
  const existing = new Set(merged.map((session) => session.id))
  for (const session of changed) {
    if (!removed.has(session.id) && !existing.has(session.id)) {
      merged.push(session)
    }
  }
  return merged
}

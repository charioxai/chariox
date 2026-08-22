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
import type { ManagedEnvironmentCatalog } from "@chariox/kernel-client/ipc-managed-environment-requests"

type WaitingRoomInventoryStatus = "loading" | "ready" | "error"
const maximumTrackedKernelInventories = 64

type WaitingRoomRowsChangedPatch = {
  inventoryVersion: string
  structuralVersion: string
  activityRevision: string
  sessions: WaitingRoomPublicSessionSummary[]
  removedSessionIds: string[]
  projects?: WaitingRoomProjectSummary[]
  removedProjectIds?: string[]
}

type WaitingRoomInventoryRefreshControllerOptions = {
  isKernelConnected: () => boolean
  getInventoryStatus: () => WaitingRoomInventoryStatus
  setInventoryStatus: (status: WaitingRoomInventoryStatus) => void
  getWaitingRoomState: () => WaitingRoomState
  getInventory: () => Promise<WaitingRoomInventory>
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
  applyRowsChanged(patch: WaitingRoomRowsChangedPatch): void
  applyRelayStatusChanged(status: RelayStatusView): void
  applyRemoteMachinesChanged(machines: RemoteMachineView[]): void
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
  let cacheScopeKey = options.getCacheScopeKey?.() ?? null
  let directTargetKernelId = currentDirectTargetKernelId(options)
  const inventoriesByKernel = new Map(
    (options.cachedInventories ?? []).map((inventory) => [inventory.kernelId, inventory]),
  )
  let pendingRefresh: Promise<void> | null = null

  if ((options.cachedInventories?.length ?? 0) > 0) {
    options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
  }

  const rememberInventory = (inventory: WaitingRoomInventory) => {
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

  const refreshNow = async () => {
    if (!options.isKernelConnected()) {
      return
    }
    if (options.getInventoryStatus() !== "ready") {
      options.setInventoryStatus("loading")
    }
    for (let attempt = 0; attempt < 2; attempt += 1) {
      refreshProjectionScope()
      const requestedCacheScopeKey = cacheScopeKey
      const requestedTargetKernelId = directTargetKernelId
      let snapshot: WaitingRoomInventory
      try {
        snapshot = await options.getInventory()
      } catch (error) {
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
        return
      }
      refreshProjectionScope()
      if (
        requestedCacheScopeKey !== cacheScopeKey
        || requestedTargetKernelId !== directTargetKernelId
      ) {
        options.setInventoryStatus("loading")
        continue
      }
      options.setInventoryStatus("ready")
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
    options.setRemoteMachines(localPresence.machines)
    options.setRemoteKernels(localPresence.kernels.filter((kernel) => (
      !options.isKernelHidden(kernel.kernel_id)
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
    options.setLaunchTarget?.(snapshot.launchTarget)
    options.setProjects?.(snapshot.projects ?? [])
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
      for (const inventory of options.loadCachedInventories?.() ?? []) {
        inventoriesByKernel.set(inventory.kernelId, inventory)
      }
    }
    directTargetKernelId = nextDirectTargetKernelId
    activeKernelId = null
    inventoryVersion = null
    inventoryInvalidated = true
    options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
  }

  return {
    applyRowsChanged(patch) {
      if (!options.isKernelConnected()) {
        return
      }
      refreshProjectionScope()
      if (patch.inventoryVersion === inventoryVersion) {
        options.reconcileWaitingRoom(options.getWaitingRoomState())
        return
      }
      const currentInventoryVersion = inventoryVersion
      inventoryVersion = patch.inventoryVersion
      options.setInventoryStatus("ready")
      const activeInventory = activeKernelId ? inventoriesByKernel.get(activeKernelId) : undefined
      if (activeInventory) {
        const sessions = mergeWaitingRoomSessionRows(
          currentInventoryVersion === null ? [] : activeInventory.sessions,
          patch.sessions.map((session) => ({
            ...session,
            kernel_id: activeInventory.kernelId,
            kernel_alias: activeInventory.kernelAlias ?? null,
            machine_id: activeInventory.machineId,
            machine_alias: activeInventory.machineAlias ?? null,
          })),
          patch.removedSessionIds,
        )
        const nextInventory = {
          ...activeInventory,
          inventoryVersion: patch.inventoryVersion,
          structuralVersion: patch.structuralVersion,
          activityRevision: patch.activityRevision,
          sessions,
          projects: patch.projects
            ? mergeWaitingRoomProjectRows(
                activeInventory.projects ?? [],
                patch.projects,
                patch.removedProjectIds ?? [],
              )
            : activeInventory.projects ?? [],
        }
        rememberInventory(nextInventory)
        options.persistInventory?.(nextInventory)
        options.setAvailableSessions(visibleSessions(inventoriesByKernel, directTargetKernelId))
        options.setProjects?.(nextInventory.projects)
      } else {
        options.setAvailableSessions(mergeWaitingRoomSessionRows(
          currentInventoryVersion === null ? [] : options.getAvailableSessions(),
          patch.sessions,
          patch.removedSessionIds,
        ))
        if (patch.projects) {
          options.setProjects?.(mergeWaitingRoomProjectRows(
            options.getProjects?.() ?? [],
            patch.projects,
            patch.removedProjectIds ?? [],
          ))
        }
      }
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRelayStatusChanged(status) {
      if (!options.isKernelConnected()) {
        return
      }
      refreshProjectionScope()
      options.setInventoryStatus("ready")
      options.setRelayStatus(status)
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRemoteMachinesChanged(machines) {
      if (!options.isKernelConnected()) {
        return
      }
      refreshProjectionScope()
      options.setInventoryStatus("ready")
      const localPresence = mergeLocalKernelPresence(
        machines,
        [],
        options.getLocalKernelPresences?.() ?? [],
        activeKernelId,
        inventoriesByKernel,
      )
      options.setRemoteMachines(localPresence.machines)
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
      (right.last_used_at_ms ?? right.created_at_ms ?? 0) - (left.last_used_at_ms ?? left.created_at_ms ?? 0)
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

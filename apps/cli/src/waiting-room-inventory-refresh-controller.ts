import {
  externalProviderSessionPageSessions,
  externalProviderSessionPageState,
} from "@arroba/kernel-client/external-provider-sessions"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import type { ExternalProviderSessionRecord, SliceRecord } from "./cli-types.js"
import type { LocalKernelPresence } from "./local-kernel-presence.js"
import { waitingRoomRemoteKernelCanDelete } from "./waiting-room-remote-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
  WaitingRoomInventory,
} from "./waiting-room-inventory-api.js"

type WaitingRoomInventoryStatus = "loading" | "ready" | "error"
const maximumTrackedKernelInventories = 64

type WaitingRoomRowsChangedPatch = {
  inventoryVersion: string
  structuralVersion: string
  activityRevision: string
  sessions: SessionListEntry[]
  removedSessionIds: string[]
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
  setRelayStatus: (status: RelayStatusView) => void
  setRemoteMachines: (machines: RemoteMachineView[]) => void
  setRemoteKernels: (kernels: RemoteKernelView[]) => void
  setTerminals: (terminals: TerminalView[]) => void
  setSlices: (slices: SliceRecord[]) => void
  setExternalProviderSessions?: (sessions: ExternalProviderSessionRecord[]) => void
  setExternalProviderSessionsPage?: (page: { hasMore: boolean; nextCursor: string | null }) => void
  reconcileWaitingRoom: (state: WaitingRoomState) => void
  warn?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
  cachedInventories?: readonly WaitingRoomInventory[]
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
  let inventoryInvalidated = false
  const inventoriesByKernel = new Map(
    (options.cachedInventories ?? []).map((inventory) => [inventory.kernelId, inventory]),
  )
  let pendingRefresh: Promise<void> | null = null

  if (inventoriesByKernel.size > 0) {
    options.setAvailableSessions(mergedCachedSessions(inventoriesByKernel.values()))
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
    const snapshot = await options.getInventory().catch((error) => {
      options.warn?.("waiting room inventory refresh failed", { error: formatError(error) })
      options.setInventoryStatus("error")
      return null
    })
    if (!snapshot) {
      return
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
    options.setAvailableSessions(mergedCachedSessions(inventoriesByKernel.values()))
    applySupplementalInventory(snapshot)
    options.reconcileWaitingRoom(options.getWaitingRoomState())
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

  return {
    applyRowsChanged(patch) {
      if (!options.isKernelConnected()) {
        return
      }
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
            kernel_alias: activeInventory.kernelAlias,
            machine_id: activeInventory.machineId,
            machine_alias: activeInventory.machineAlias,
          })),
          patch.removedSessionIds,
        )
        const nextInventory = {
          ...activeInventory,
          inventoryVersion: patch.inventoryVersion,
          structuralVersion: patch.structuralVersion,
          activityRevision: patch.activityRevision,
          sessions,
        }
        rememberInventory(nextInventory)
        options.persistInventory?.(nextInventory)
        options.setAvailableSessions(mergedCachedSessions(inventoriesByKernel.values()))
      } else {
        options.setAvailableSessions(mergeWaitingRoomSessionRows(
          currentInventoryVersion === null ? [] : options.getAvailableSessions(),
          patch.sessions,
          patch.removedSessionIds,
        ))
      }
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRelayStatusChanged(status) {
      if (!options.isKernelConnected()) {
        return
      }
      options.setInventoryStatus("ready")
      options.setRelayStatus(status)
      options.reconcileWaitingRoom(options.getWaitingRoomState())
    },
    applyRemoteMachinesChanged(machines) {
      if (!options.isKernelConnected()) {
        return
      }
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
      inventoryVersion = null
      inventoryInvalidated = true
    },
  }
}

function mergedCachedSessions(inventories: Iterable<WaitingRoomInventory>): SessionListEntry[] {
  return Array.from(inventories)
    .flatMap((inventory) => inventory.sessions)
    .sort((left, right) => (
      (right.last_used_at_ms ?? right.created_at_ms) - (left.last_used_at_ms ?? left.created_at_ms)
    ))
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
      kernel_alias: presence.kernelAlias,
      machine_id: presence.machineId,
      machine_alias: presence.machineAlias,
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
          machine_alias: presence.machineAlias,
          display_name: presence.machineAlias ?? presence.machineId,
          trust_status: "approved",
          online: true,
          pending: false,
          kernel_count: localKernelCount,
        })
  }
  return { machines: [...machinesById.values()], kernels: [...kernelsById.values()] }
}

function mergeWaitingRoomSessionRows(
  current: SessionListEntry[],
  changed: SessionListEntry[],
  removedIds: string[],
): SessionListEntry[] {
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

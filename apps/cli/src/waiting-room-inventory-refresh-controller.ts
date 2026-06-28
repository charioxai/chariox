import { externalProviderSessionsSorted } from "@arroba/kernel-client/external-provider-sessions"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import type { ExternalProviderSessionRecord, SliceRecord } from "./cli-types.js"
import { waitingRoomRemoteKernelCanDelete } from "./waiting-room-remote-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
  WaitingRoomInventory,
} from "./waiting-room-inventory-api.js"

type WaitingRoomInventoryStatus = "loading" | "ready" | "error"

type WaitingRoomRowsChangedPatch = {
  inventoryVersion: string
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
  let pendingRefresh: Promise<void> | null = null

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
    if (snapshot.inventoryVersion === inventoryVersion) {
      applySupplementalInventory(snapshot)
      options.reconcileWaitingRoom(options.getWaitingRoomState())
      return
    }

    inventoryVersion = snapshot.inventoryVersion
    options.setAvailableSessions(snapshot.sessions)
    applySupplementalInventory(snapshot)
    options.reconcileWaitingRoom(options.getWaitingRoomState())
  }

  const applySupplementalInventory = (snapshot: WaitingRoomInventory) => {
    options.setRelayStatus(snapshot.relayStatus)
    options.setRemoteMachines(snapshot.remoteMachines)
    options.setRemoteKernels(snapshot.remoteKernels.filter((kernel) => (
      !options.isKernelHidden(kernel.kernel_id)
      || !waitingRoomRemoteKernelCanDelete(kernel)
    )))
    options.setTerminals(snapshot.terminals)
    options.setSlices(snapshot.slices)
    options.setExternalProviderSessions?.(externalProviderSessionsSorted(snapshot.externalProviderSessions))
    options.setExternalProviderSessionsPage?.({
      hasMore: snapshot.externalProviderSessionsHasMore ?? false,
      nextCursor: snapshot.externalProviderSessionsNextCursor ?? null,
    })
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
      options.setAvailableSessions(
        mergeWaitingRoomSessionRows(
          currentInventoryVersion === null ? [] : options.getAvailableSessions(),
          patch.sessions,
          patch.removedSessionIds,
        ),
      )
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
      options.setRemoteMachines(machines)
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
    },
  }
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

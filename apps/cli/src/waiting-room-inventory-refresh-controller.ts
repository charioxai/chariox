import type { RelayStatusView, TerminalView } from "./relay-api.js"
import type { SessionListEntry } from "./sessions.js"
import type { SliceRecord } from "./cli-types.js"
import { waitingRoomRemoteKernelCanDelete } from "./waiting-room-remote-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
  WaitingRoomInventory,
} from "./waiting-room-inventory-api.js"

type WaitingRoomInventoryStatus = "loading" | "ready" | "error"

type WaitingRoomInventoryRefreshControllerOptions = {
  isKernelConnected: () => boolean
  getInventoryStatus: () => WaitingRoomInventoryStatus
  setInventoryStatus: (status: WaitingRoomInventoryStatus) => void
  getWaitingRoomState: () => WaitingRoomState
  getInventory: () => Promise<WaitingRoomInventory>
  isKernelHidden: (kernelId: string) => boolean
  setAvailableSessions: (sessions: SessionListEntry[]) => void
  setRelayStatus: (status: RelayStatusView) => void
  setRemoteMachines: (machines: RemoteMachineView[]) => void
  setRemoteKernels: (kernels: RemoteKernelView[]) => void
  setTerminals: (terminals: TerminalView[]) => void
  setSlices: (slices: SliceRecord[]) => void
  reconcileWaitingRoom: (state: WaitingRoomState) => void
  warn?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type WaitingRoomInventoryRefreshController = {
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
      options.reconcileWaitingRoom(options.getWaitingRoomState())
      return
    }

    inventoryVersion = snapshot.inventoryVersion
    options.setAvailableSessions(snapshot.sessions)
    options.setRelayStatus(snapshot.relayStatus)
    options.setRemoteMachines(snapshot.remoteMachines)
    options.setRemoteKernels(snapshot.remoteKernels.filter((kernel) => (
      !options.isKernelHidden(kernel.kernel_id)
      || !waitingRoomRemoteKernelCanDelete(kernel)
    )))
    options.setTerminals(snapshot.terminals)
    options.setSlices(snapshot.slices)
    options.reconcileWaitingRoom(options.getWaitingRoomState())
  }

  return {
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

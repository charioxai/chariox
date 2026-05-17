import type {
  WaitingRoomRemoteKernelView,
  WaitingRoomRemoteMachineView,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import { formatSessionDisplayLabel, type SessionListEntry } from "./sessions.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import type { WaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"
import {
  deriveWaitingRoomDeleteDecision,
  deriveWaitingRoomSessionLifecycleDecision,
  type WaitingRoomSessionLifecycleAction,
} from "./waiting-room-controller.js"

type SessionLifecycleResult = {
  id: string
  alias?: string | null
}

export type WaitingRoomLifecycleActionControllerDeps = {
  isKernelConnected: () => boolean
  connectDetachedKernel: () => Promise<void>
  getWaitingRoomState: () => WaitingRoomState
  getRemoteState: () => WaitingRoomRemoteState
  getAvailableSessions: () => SessionListEntry[]
  setAvailableSessions: (sessions: SessionListEntry[]) => void
  getProviderCatalog: () => ProviderCatalog
  getWorkspaceTarget: () => string
  confirmationController: WaitingRoomLifecycleConfirmationController
  archiveSessionById: (sessionId: string) => Promise<SessionLifecycleResult>
  deleteSessionByRef: (sessionRef: string, workspace: string) => Promise<SessionLifecycleResult>
  forgetRemoteMachine: (machineRef: string) => Promise<{ machine_id?: string | null }>
  getRemoteMachines: () => WaitingRoomRemoteMachineView[]
  setRemoteMachines: (machines: WaitingRoomRemoteMachineView[]) => void
  getRemoteKernels: () => WaitingRoomRemoteKernelView[]
  setRemoteKernels: (kernels: WaitingRoomRemoteKernelView[]) => void
  hideRemoteKernel: (kernelId: string) => void
  invalidateInventory: () => void
  reconcileWaitingRoom: (state: WaitingRoomState) => void
  refreshWaitingRoomData: () => Promise<void>
  sessionBrowserOpen: () => boolean
  closeSessionBrowserDialog: () => void
  flashFooter: (message: string, tone: "info" | "error") => void
  warn?: (message: string, fields: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
}

export type WaitingRoomLifecycleActionController = {
  applyAction(action: WaitingRoomSessionLifecycleAction, stateOverride?: WaitingRoomState): Promise<void>
}

export function createWaitingRoomLifecycleActionController(
  deps: WaitingRoomLifecycleActionControllerDeps,
): WaitingRoomLifecycleActionController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const removeSessions = (removedIds: Set<string>) => {
    deps.setAvailableSessions(deps.getAvailableSessions().filter((candidate) => !removedIds.has(candidate.id)))
  }

  const refreshAfterSessionMutation = async (state: WaitingRoomState = deps.getWaitingRoomState()) => {
    deps.invalidateInventory()
    deps.reconcileWaitingRoom(state)
    await deps.refreshWaitingRoomData()
  }

  const closeSessionBrowserIfOpen = () => {
    if (deps.sessionBrowserOpen()) {
      deps.closeSessionBrowserDialog()
    }
  }

  return {
    async applyAction(action, stateOverride) {
      try {
        if (!deps.isKernelConnected()) {
          await deps.connectDetachedKernel()
        }
        const effectiveState = stateOverride ?? deps.getWaitingRoomState()
        const decision = action === "delete"
          ? deriveWaitingRoomDeleteDecision({
              state: effectiveState,
              sessions: deps.getAvailableSessions(),
              catalog: deps.getProviderCatalog(),
              remote: deps.getRemoteState(),
            })
          : deriveWaitingRoomSessionLifecycleDecision({
              action,
              state: effectiveState,
              sessions: deps.getAvailableSessions(),
              catalog: deps.getProviderCatalog(),
            })
        if (decision.action === "error") {
          deps.confirmationController.clear()
          deps.flashFooter(decision.message, "error")
          return
        }

        const confirmation = deps.confirmationController.confirm(action, decision)
        if (confirmation.action === "await-confirmation") {
          deps.flashFooter(confirmation.message, confirmation.tone)
          return
        }

        if (decision.action === "archive") {
          const updated = await deps.archiveSessionById(decision.session.id)
          removeSessions(new Set([updated.id]))
          await refreshAfterSessionMutation()
          deps.flashFooter(`archived session ${formatSessionDisplayLabel(updated)}`, "info")
          return
        }
        if (decision.action === "archive-all") {
          const archived = []
          for (const session of decision.sessions) {
            archived.push(await deps.archiveSessionById(session.id))
          }
          removeSessions(new Set(archived.map((session) => session.id)))
          await refreshAfterSessionMutation({ ...deps.getWaitingRoomState(), focus: "new", sessionIndex: 0 })
          closeSessionBrowserIfOpen()
          deps.flashFooter(`archived ${archived.length} session${archived.length === 1 ? "" : "s"}`, "info")
          return
        }
        if (decision.action === "delete-session") {
          const updated = await deps.deleteSessionByRef(decision.session.id, deps.getWorkspaceTarget())
          removeSessions(new Set([updated.id]))
          await refreshAfterSessionMutation()
          deps.flashFooter(`deleted session ${formatSessionDisplayLabel(updated)}`, "error")
          return
        }
        if (decision.action === "delete-all-sessions") {
          const deleted = []
          for (const session of decision.sessions) {
            deleted.push(await deps.deleteSessionByRef(session.id, deps.getWorkspaceTarget()))
          }
          removeSessions(new Set(deleted.map((session) => session.id)))
          await refreshAfterSessionMutation({ ...deps.getWaitingRoomState(), focus: "new", sessionIndex: 0 })
          closeSessionBrowserIfOpen()
          deps.flashFooter(`deleted ${deleted.length} session${deleted.length === 1 ? "" : "s"}`, "error")
          return
        }
        if (decision.action === "delete") {
          const updated = await deps.deleteSessionByRef(decision.session.id, deps.getWorkspaceTarget())
          removeSessions(new Set([updated.id]))
          await refreshAfterSessionMutation()
          deps.flashFooter(`deleted session ${formatSessionDisplayLabel(updated)}`, "error")
          return
        }
        if (decision.action === "delete-machine") {
          const deleted = await deps.forgetRemoteMachine(decision.machineId)
          const deletedMachineId = deleted.machine_id || decision.machineId
          deps.setRemoteMachines(deps.getRemoteMachines().filter((machine) => machine.machine_id !== deletedMachineId))
          deps.setRemoteKernels(deps.getRemoteKernels().filter((kernel) => kernel.machine_id !== deletedMachineId))
          await refreshAfterSessionMutation()
          deps.flashFooter(`deleted machine ${decision.label}`, "error")
          return
        }
        if (decision.action === "delete-kernel") {
          deps.hideRemoteKernel(decision.kernelId)
          deps.setRemoteKernels(deps.getRemoteKernels().filter((kernel) => kernel.kernel_id !== decision.kernelId))
          deps.reconcileWaitingRoom(deps.getWaitingRoomState())
          deps.flashFooter(`deleted kernel ${decision.label}`, "error")
        }
      } catch (error) {
        deps.warn?.("waiting room session lifecycle action failed", {
          action,
          error: formatError(error),
        })
        deps.flashFooter(formatError(error), "error")
      }
    },
  }
}

import { formatSessionDisplayLabel } from "./sessions.js"
import type {
  WaitingRoomDeleteDecision,
  WaitingRoomSessionLifecycleAction,
  WaitingRoomSessionLifecycleDecision,
} from "./waiting-room-controller.js"

const DEFAULT_CONFIRMATION_WINDOW_MS = 4_000

type WaitingRoomLifecycleDecision = WaitingRoomSessionLifecycleDecision | WaitingRoomDeleteDecision

type WaitingRoomLifecycleTarget = {
  kind: "session" | "sessions" | "machine" | "kernel" | "slice"
  id: string
  label: string
  verb: "archive" | "delete"
}

type PendingWaitingRoomLifecycleAction = {
  action: WaitingRoomSessionLifecycleAction
  targetKind: WaitingRoomLifecycleTarget["kind"]
  targetId: string
  expiresAtMs: number
}

export type WaitingRoomLifecycleConfirmationResult =
  | { action: "confirmed"; target: WaitingRoomLifecycleTarget }
  | { action: "await-confirmation"; target: WaitingRoomLifecycleTarget; message: string; tone: "info" | "error" }

type WaitingRoomLifecycleConfirmationControllerOptions = {
  now?: () => number
  confirmationWindowMs?: number
}

export type WaitingRoomLifecycleConfirmationController = {
  confirm(
    action: WaitingRoomSessionLifecycleAction,
    decision: WaitingRoomLifecycleDecision,
  ): WaitingRoomLifecycleConfirmationResult
  clear(): void
  pending(): PendingWaitingRoomLifecycleAction | null
}

export function createWaitingRoomLifecycleConfirmationController(
  options: WaitingRoomLifecycleConfirmationControllerOptions = {},
): WaitingRoomLifecycleConfirmationController {
  const now = options.now ?? Date.now
  const confirmationWindowMs = options.confirmationWindowMs ?? DEFAULT_CONFIRMATION_WINDOW_MS
  let pending: PendingWaitingRoomLifecycleAction | null = null

  return {
    confirm(action, decision) {
      const target = waitingRoomLifecycleTarget(decision)
      const currentTime = now()
      const existing = pending
      if (
        !existing
        || existing.action !== action
        || existing.targetKind !== target.kind
        || existing.targetId !== target.id
        || existing.expiresAtMs <= currentTime
      ) {
        pending = {
          action,
          targetKind: target.kind,
          targetId: target.id,
          expiresAtMs: currentTime + confirmationWindowMs,
        }
        const keyLabel = action === "archive" ? "A" : "D"
        return {
          action: "await-confirmation",
          target,
          message: `press ${keyLabel} again to ${target.verb} ${target.label}`,
          tone: action === "delete" ? "error" : "info",
        }
      }

      pending = null
      return { action: "confirmed", target }
    },
    clear() {
      pending = null
    },
    pending() {
      return pending
    },
  }
}

function waitingRoomLifecycleTarget(
  decision: WaitingRoomLifecycleDecision,
): WaitingRoomLifecycleTarget {
  if (decision.action === "archive") {
    return {
      kind: "session",
      id: decision.session.id,
      label: `session ${formatSessionDisplayLabel(decision.session)}`,
      verb: "archive",
    }
  }
  if (decision.action === "archive-all") {
    return {
      kind: "sessions",
      id: "all",
      label: `${decision.sessions.length} session${decision.sessions.length === 1 ? "" : "s"}`,
      verb: "archive",
    }
  }
  if (decision.action === "delete-session" || decision.action === "delete") {
    return {
      kind: "session",
      id: decision.session.id,
      label: `session ${formatSessionDisplayLabel(decision.session)}`,
      verb: "delete",
    }
  }
  if (decision.action === "delete-all-sessions") {
    return {
      kind: "sessions",
      id: "all",
      label: `${decision.sessions.length} session${decision.sessions.length === 1 ? "" : "s"}`,
      verb: "delete",
    }
  }
  if (decision.action === "delete-machine") {
    return {
      kind: "machine",
      id: decision.machineId,
      label: `machine ${decision.label}`,
      verb: "delete",
    }
  }
  if (decision.action === "delete-kernel") {
    return {
      kind: "kernel",
      id: decision.kernelId,
      label: `kernel ${decision.label}`,
      verb: "delete",
    }
  }
  if (decision.action === "delete-slice") {
    return {
      kind: "slice",
      id: decision.sliceId,
      label: `slice ${decision.label}`,
      verb: "delete",
    }
  }
  throw new Error("unsupported waiting room lifecycle decision")
}

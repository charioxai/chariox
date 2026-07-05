import {
  normalizeRuntimeSession,
  type RuntimeSession,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { DEFAULT_CONNECTED_STATUS } from "./runtime.js"
import {
  sessionResponseLayout,
} from "@arroba/kernel-client/session-config-projection"
import {
  sessionProjectedStreamingAgentId,
} from "@arroba/kernel-client/session-prompt-work"
import {
  sessionPromptLifecycleTransition,
} from "@arroba/kernel-client/session-prompt-lifecycle"
import {
  sessionShouldConfirmIdleTurnCompletion,
  sessionRuntimeTransitionState,
} from "@arroba/kernel-client/session-runtime-transition"

export type SessionStateApplyTurnCompletion = {
  reset(): void
  isConfirmed(): boolean
  confirm(): void
  confirmAndSchedule(): void
}

export type SessionStateApplyPromptStop = {
  reset(): void
}

export type SessionStateApplyControllerDeps = {
  getSession: () => RuntimeSession
  setSession: (session: RuntimeSession) => void
  getFocusedAgentId: () => string | null
  getCurrentResponseLayout: () => MultiAgentResponseLayout
  getLayoutPreference: () => MultiAgentResponseLayout | null | undefined
  setResponseLayout: (layout: MultiAgentResponseLayout) => void
  getWorking: () => boolean
  setWorking: (working: boolean) => void
  getSubmitting: () => boolean
  setSubmitting: (submitting: boolean) => void
  clearSubmittingAgentId: () => void
  getAgentBusyLatches: () => Record<string, boolean>
  setAgentActivityLabels: (labels: Record<string, string | null>) => void
  getAgentActivityLabels: () => Record<string, string | null>
  clearAgentBusy: (agentId: string) => void
  getStreamingAgentId: () => string | null
  setStreamingAgentId: (agentId: string | null) => void
  getProviderActivityLabel: () => string | null
  setProviderActivityLabel: (label: string | null) => void
  getActiveStatusLabel: () => string | null
  setActiveStatusLabel: (label: string | null) => void
  getStatusLine: () => string
  setStatusLine: (line: string) => void
  clearActiveToolLabels: () => void
  turnCompletion: SessionStateApplyTurnCompletion
  cancelPendingTurnCompletion: () => void
  promptStop: SessionStateApplyPromptStop
  syncQueuedPromptEntries: (session: RuntimeSession) => void
  syncVisibleActivityLabel: () => void
  updateSessionChrome: () => void
  refreshSplitPaneFocusRepaint: () => void
}

export function createSessionStateApplyController(
  deps: SessionStateApplyControllerDeps,
) {
  const apply = (incomingSession: RuntimeSession) => {
    const nextSession = normalizeRuntimeSession(incomingSession)
    const currentSession = deps.getSession()
    const previousFocusedAgentId = deps.getFocusedAgentId()
    const previousLayout = deps.getCurrentResponseLayout()
    const promptLifecycle = sessionPromptLifecycleTransition(currentSession, nextSession)
    const transition = sessionRuntimeTransitionState({
      currentSession,
      nextSession,
      currentWorking: deps.getWorking(),
      currentStreamingAgentId: deps.getStreamingAgentId(),
      currentAgentActivityLabels: deps.getAgentActivityLabels(),
    })
    const nextLayout = sessionResponseLayout(nextSession, deps.getLayoutPreference())
    const shouldConfirmIdleCompletion = sessionShouldConfirmIdleTurnCompletion({
      nextSession,
      currentWorking: deps.getWorking(),
      currentSubmitting: deps.getSubmitting(),
      currentBusyLatches: deps.getAgentBusyLatches(),
      currentStreamingAgentId: deps.getStreamingAgentId(),
      currentProviderActivityLabel: deps.getProviderActivityLabel(),
      currentActiveStatusLabel: deps.getActiveStatusLabel(),
    })

    deps.setSession(nextSession)
    deps.syncQueuedPromptEntries(nextSession)
    deps.setAgentActivityLabels(transition.nextAgentActivityLabels)
    deps.setStreamingAgentId(transition.nextStreamingAgentId)
    deps.setResponseLayout(nextLayout)
    deps.setWorking(transition.nextWorking)

    if (transition.nextHasPromptWork) {
      deps.turnCompletion.reset()
    } else if (deps.turnCompletion.isConfirmed() || shouldConfirmIdleCompletion) {
      deps.turnCompletion.confirmAndSchedule()
    } else {
      deps.cancelPendingTurnCompletion()
    }

    deps.setProviderActivityLabel(transition.nextFocusedActivityLabel)
    deps.setActiveStatusLabel(transition.nextFocusedActivityLabel)

    if (promptLifecycle.activePromptChanged) {
      deps.setSubmitting(false)
      deps.clearSubmittingAgentId()
      deps.promptStop.reset()
    }
    for (const settledAgentId of promptLifecycle.settledAgentIds) {
      deps.clearAgentBusy(settledAgentId)
    }
    if (promptLifecycle.settledAgentIds.length > 0 && !transition.nextHasPromptWork) {
      deps.setWorking(false)
    }
    if (promptLifecycle.cancelledPromptSettled) {
      deps.clearActiveToolLabels()
      deps.setAgentActivityLabels({})
      deps.setStreamingAgentId(sessionProjectedStreamingAgentId(nextSession))
      deps.setProviderActivityLabel(null)
      deps.setActiveStatusLabel(null)
      if (deps.getStatusLine() === "Cancellation requested.") {
        deps.setStatusLine(DEFAULT_CONNECTED_STATUS)
      }
      if (!transition.nextHasPromptWork) {
        deps.turnCompletion.confirm()
        deps.cancelPendingTurnCompletion()
        deps.setWorking(false)
      }
    }
    if (!transition.nextHasPromptWork) {
      deps.setSubmitting(false)
      deps.promptStop.reset()
    }

    deps.syncVisibleActivityLabel()
    deps.updateSessionChrome()

    if (
      nextLayout === "split"
      && (previousLayout !== nextLayout
        || previousFocusedAgentId !== transition.nextFocusedAgentId
        || transition.previousAgentSignature !== transition.nextAgentSignature)
    ) {
      deps.refreshSplitPaneFocusRepaint()
    }

    return nextSession
  }

  return {
    apply,
  }
}

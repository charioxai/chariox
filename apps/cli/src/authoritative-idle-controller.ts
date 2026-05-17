import type { RuntimeSession } from "./cli-types.js"
import { DEFAULT_CONNECTED_STATUS } from "./runtime.js"
import {
  sessionHasProcessingAgent,
  sessionHasPromptWork,
} from "./session-state.js"

export type AuthoritativeIdleControllerDeps = {
  batchUpdate: (callback: () => void) => void
  resetTurnCompletion: () => void
  clearActiveToolLabels: () => void
  setAgentActivityLabels: (labels: Record<string, string | null>) => void
  setStreamingAgentId: (agentId: string | null) => void
  setSubmitting: (submitting: boolean) => void
  clearSubmittingAgentId: () => void
  resetPromptStop: () => void
  setAgentBusyLatches: (latches: Record<string, boolean>) => void
  setProviderActivityLabel: (label: string | null) => void
  setActiveStatusLabel: (label: string | null) => void
  setWorking: (working: boolean) => void
  getStatusLine: () => string
  setStatusLine: (statusLine: string) => void
  renderSessionChromeBoundary: () => void
}

export function createAuthoritativeIdleController(
  deps: AuthoritativeIdleControllerDeps,
) {
  const clear = (nextSession: RuntimeSession) => {
    if (sessionHasPromptWork(nextSession) || sessionHasProcessingAgent(nextSession)) {
      return false
    }

    deps.batchUpdate(() => {
      deps.resetTurnCompletion()
      deps.clearActiveToolLabels()
      deps.setAgentActivityLabels({})
      deps.setStreamingAgentId(null)
      deps.setSubmitting(false)
      deps.clearSubmittingAgentId()
      deps.resetPromptStop()
      deps.setAgentBusyLatches({})
      deps.setProviderActivityLabel(null)
      deps.setActiveStatusLabel(null)
      deps.setWorking(false)
      if (deps.getStatusLine() === "Cancellation requested.") {
        deps.setStatusLine(DEFAULT_CONNECTED_STATUS)
      }
    })
    deps.renderSessionChromeBoundary()
    return true
  }

  return {
    clear,
  }
}

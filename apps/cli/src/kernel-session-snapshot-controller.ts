import type {
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import {
  sessionHasPromptWork,
  sessionPromptWorkJustCompleted,
} from "@arroba/kernel-client/session-prompt-work"

type KernelSessionSnapshotControllerDeps = {
  getSession: () => RuntimeSession
  getProviderRun: () => RuntimeProviderRun | null
  projectSession: (
    session: RuntimeSession,
    providerRun: RuntimeProviderRun | null,
  ) => RuntimeSession
  shouldRefreshAgentPanesForSessionChange: (session: RuntimeSession) => boolean
  applySessionState: (session: RuntimeSession) => void
  sameProviderRun: (left: RuntimeProviderRun, right: RuntimeProviderRun) => boolean
  logProviderRunDebug: (
    message: string,
    run: RuntimeProviderRun | null,
    fields?: Record<string, unknown>,
  ) => void
  setProviderRun: (run: RuntimeProviderRun | null) => void
  updateSessionChrome: () => void
  supportsKernelEventStream: () => boolean
  recoverProviderRun: (reason: string) => unknown
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
}

export function createKernelSessionSnapshotController(
  deps: KernelSessionSnapshotControllerDeps,
) {
  const apply = async (
    nextSession: RuntimeSession,
    nextProviderRun: RuntimeProviderRun | null,
  ) => {
    const previousSession = deps.getSession()
    const projectedSession = deps.projectSession(nextSession, nextProviderRun ?? deps.getProviderRun())
    const shouldRefreshPanes = deps.shouldRefreshAgentPanesForSessionChange(projectedSession)
    const promptJustCompleted = sessionPromptWorkJustCompleted(previousSession, projectedSession)

    deps.applySessionState(projectedSession)

    const activeRun = deps.getProviderRun()
    if (nextProviderRun) {
      if (!activeRun || !deps.sameProviderRun(activeRun, nextProviderRun)) {
        deps.logProviderRunDebug("kernel event refreshed provider run", nextProviderRun, {
          session_id: nextSession.id,
          previous_provider_run_id: activeRun?.id ?? null,
        })
        deps.setProviderRun(nextProviderRun)
        deps.updateSessionChrome()
      }
    } else if (activeRun) {
      deps.logProviderRunDebug("kernel event cleared provider run", activeRun, {
        session_id: nextSession.id,
      })
      deps.setProviderRun(null)
      deps.updateSessionChrome()
      if (!deps.supportsKernelEventStream() && sessionHasPromptWork(projectedSession)) {
        void deps.recoverProviderRun("missing active provider run")
      }
    }

    if (shouldRefreshPanes || promptJustCompleted) {
      await deps.refreshAgentPanes(projectedSession)
    }
  }

  return { apply }
}

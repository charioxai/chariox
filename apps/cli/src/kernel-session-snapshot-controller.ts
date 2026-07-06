import type {
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import {
  sessionSnapshotRefreshTransition,
} from "@arroba/kernel-client/session-runtime-transition"
import { sessionShouldRecoverMissingActiveProviderRun } from "@arroba/kernel-client/provider-run-recovery"

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
    const refreshTransition = sessionSnapshotRefreshTransition({
      previousSession,
      nextSession: projectedSession,
      sessionChangeRequiresPaneRefresh:
        deps.shouldRefreshAgentPanesForSessionChange(projectedSession),
    })

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
      if (!deps.supportsKernelEventStream() && sessionShouldRecoverMissingActiveProviderRun(projectedSession)) {
        void deps.recoverProviderRun("missing active provider run")
      }
    }

    if (refreshTransition.shouldRefreshAgentPanes) {
      await deps.refreshAgentPanes(projectedSession)
    }
  }

  return { apply }
}

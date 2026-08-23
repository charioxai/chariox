import type {
  RuntimeProviderRun,
  RuntimeSession,
  WorkflowRun,
} from "./cli-types.js"
import {
  sessionSnapshotRefreshTransition,
} from "@chariox/kernel-client/session-runtime-transition"
import { sessionShouldRecoverMissingActiveProviderRun } from "@chariox/kernel-client/provider-run-recovery"

const MAX_OBSERVED_TERMINAL_WORKFLOW_RUNS = 100

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
    const mergedSession = {
      ...projectedSession,
      workflow_runs: mergeObservedTerminalWorkflowRuns(
        previousSession.workflow_runs ?? [],
        projectedSession.workflow_runs ?? [],
      ),
    }
    const refreshTransition = sessionSnapshotRefreshTransition({
      previousSession,
      nextSession: mergedSession,
      sessionChangeRequiresPaneRefresh:
        deps.shouldRefreshAgentPanesForSessionChange(mergedSession),
    })

    deps.applySessionState(mergedSession)

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
      await deps.refreshAgentPanes(mergedSession)
    }
  }

  return { apply }
}

function mergeObservedTerminalWorkflowRuns(
  previousRuns: WorkflowRun[],
  incomingRuns: WorkflowRun[],
): WorkflowRun[] {
  const incomingIds = new Set(incomingRuns.map((run) => run.id))
  const observedTerminalRuns = previousRuns
    .filter((run) => isTerminalWorkflowRunStatus(run.status) && !incomingIds.has(run.id))
    .sort((left, right) => right.created_at_ms - left.created_at_ms)
    .slice(0, MAX_OBSERVED_TERMINAL_WORKFLOW_RUNS)
  return [...incomingRuns, ...observedTerminalRuns]
}

function isTerminalWorkflowRunStatus(status: string) {
  return status === "Completed" || status === "Failed" || status === "Stopped"
}

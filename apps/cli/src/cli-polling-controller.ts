import type {
  RuntimeAttachment,
  RuntimeNoticeRecord,
  RuntimeProviderRun,
  RuntimeSession,
  TerminalOutputRecord,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import type { CharioxLogger } from "./logging.js"
import { runPollingLoop as defaultRunPollingLoop } from "./polling-effects.js"
import {
  sessionSnapshotRefreshTransition,
} from "@chariox/kernel-client/session-runtime-transition"
import {
  sessionCanIgnoreMissingActiveProviderRun,
  sessionShouldRecoverMissingActiveProviderRun,
} from "@chariox/kernel-client/provider-run-recovery"
import { runtimeNoticeShouldRenderInAgentPane } from "./runtime-notice-filter.js"
import { workflowRuntimeSignature } from "./workflow-runtime-signature.js"

type PollLoop = typeof defaultRunPollingLoop

type CliPollingControllerDeps = {
  runPollingLoop?: PollLoop
  isClosing: () => boolean
  logger?: CharioxLogger | null
  formatError: (error: unknown) => string
  isSessionUnavailableError: (error: unknown) => boolean
  getPollRecoveryDecision: Parameters<PollLoop>[0]["getPollRecoveryDecision"]
  onSessionUnavailable: () => unknown | Promise<unknown>
  onMarkRecovered: Parameters<PollLoop>[0]["onMarkRecovered"]
  onMarkDegraded: Parameters<PollLoop>[0]["onMarkDegraded"]
  onFatalError: (error: unknown) => unknown | Promise<unknown>
  sleep: (ms: number) => Promise<void>

  isAttached: () => boolean
  getAttachment: () => RuntimeAttachment | null
  getSession: () => RuntimeSession
  workflowScreenActive: () => boolean
  rebuildTranscript: () => void
  getProviderRun: () => RuntimeProviderRun | null
  setProviderRun: (run: RuntimeProviderRun | null) => void
  updateSessionChrome: () => void
  recordDaemonActivity: (activityType: string) => void
  queueTerminalOutputRecords: (records: TerminalOutputRecord[]) => void
  pumpTerminalOutput: (sessionId: string, attachmentId: string) => Promise<TerminalOutputRecord[]>
  pollRuntimeNotices: (sessionId: string, attachmentId: string) => Promise<RuntimeNoticeRecord[]>
  synchronizeRoomEnvironmentActivity: () => Promise<unknown>
  appendNotice: (message: string) => void
  getSessionState: (sessionId: string) => Promise<RuntimeSession>
  getWorkspaceLiveSyncStatus?: (sessionId: string) => Promise<WorkspaceLiveSyncStatus>
  setWorkspaceLiveSyncStatus?: (status: WorkspaceLiveSyncStatus | null) => void
  projectSession: (
    session: RuntimeSession,
    providerRun: RuntimeProviderRun | null,
  ) => RuntimeSession
  shouldRefreshAgentPanesForSessionChange: (session: RuntimeSession) => boolean
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  tryGetProviderRun: (providerRunId: string) => Promise<RuntimeProviderRun | null>
  sameProviderRun: (left: RuntimeProviderRun, right: RuntimeProviderRun) => boolean
  logProviderRunDebug: (
    message: string,
    run: RuntimeProviderRun | null,
    fields?: Record<string, unknown>,
  ) => void
  recoverProviderRun: (reason: string) => unknown
}

export function createCliPollingController(deps: CliPollingControllerDeps) {
  const runPollingLoop = deps.runPollingLoop ?? defaultRunPollingLoop
  const commonOptions = {
    isClosing: deps.isClosing,
    logger: deps.logger ?? null,
    formatError: deps.formatError,
    isSessionUnavailableError: deps.isSessionUnavailableError,
    getPollRecoveryDecision: deps.getPollRecoveryDecision,
    onSessionUnavailable: deps.onSessionUnavailable,
    onMarkRecovered: deps.onMarkRecovered,
    onMarkDegraded: deps.onMarkDegraded,
    onFatalError: deps.onFatalError,
    sleep: deps.sleep,
  }

  const pollOutput = async () => {
    await runPollingLoop({
      ...commonOptions,
      operation: "polling terminal output",
      intervalMs: 50,
      task: async () => {
        const attachment = deps.getAttachment()
        if (!attachment) {
          return
        }
        let records: TerminalOutputRecord[]
        try {
          records = await deps.pumpTerminalOutput(deps.getSession().id, attachment.id)
        } catch (error) {
          const message = deps.formatError(error)
          if (
            /has no active provider run/i.test(message)
            && sessionCanIgnoreMissingActiveProviderRun(deps.getSession())
          ) {
            deps.setProviderRun(null)
            deps.updateSessionChrome()
            return
          }
          throw error
        }
        if (records.length > 0) {
          deps.recordDaemonActivity("terminal_output")
        }
        deps.queueTerminalOutputRecords(records)
      },
    })
  }

  const pollNotices = async () => {
    await runPollingLoop({
      ...commonOptions,
      operation: "polling runtime notices",
      intervalMs: 150,
      task: async () => {
        const attachment = deps.getAttachment()
        if (!attachment) {
          return
        }
        const notices = await deps.pollRuntimeNotices(deps.getSession().id, attachment.id)
        deps.recordDaemonActivity("runtime_notices")
        for (const notice of notices) {
          if (!runtimeNoticeShouldRenderInAgentPane(notice.message)) {
            continue
          }
          deps.appendNotice(notice.message)
        }
      },
    })
  }

  const pollSessionState = async () => {
    await runPollingLoop({
      ...commonOptions,
      operation: "polling session state",
      intervalMs: 250,
      task: async () => {
        if (!deps.isAttached()) {
          return
        }
        const previousSession = deps.getSession()
        const shouldRefreshWorkflowOutline = deps.workflowScreenActive()
        const previousWorkflowRuntimeSignature = shouldRefreshWorkflowOutline
          ? workflowRuntimeSignature(previousSession)
          : null
        const session = await deps.getSessionState(previousSession.id)
        deps.recordDaemonActivity("session_state_poll")
        const projectedSession = deps.projectSession(session, deps.getProviderRun())
        const refreshTransition = sessionSnapshotRefreshTransition({
          previousSession,
          nextSession: projectedSession,
          sessionChangeRequiresPaneRefresh:
            deps.shouldRefreshAgentPanesForSessionChange(projectedSession),
        })
        deps.applySessionState(projectedSession)
        if (
          shouldRefreshWorkflowOutline
          && workflowRuntimeSignature(projectedSession) !== previousWorkflowRuntimeSignature
        ) {
          deps.rebuildTranscript()
        }
        if (refreshTransition.shouldRefreshAgentPanes) {
          await deps.refreshAgentPanes(projectedSession)
        }
        if (
          refreshTransition.shouldRefreshWorkspaceLiveSyncStatus
          && deps.getWorkspaceLiveSyncStatus
          && deps.setWorkspaceLiveSyncStatus
        ) {
          try {
            deps.setWorkspaceLiveSyncStatus(await deps.getWorkspaceLiveSyncStatus(projectedSession.id))
            deps.updateSessionChrome()
          } catch (error) {
            deps.logger?.warn("workspace live sync status refresh failed", {
              error: deps.formatError(error),
              session_id: projectedSession.id,
            })
          }
        }
        if (session.active_provider_run_id) {
          const activeRun = deps.getProviderRun()
          const run = await deps.tryGetProviderRun(session.active_provider_run_id)
          if (run && (!activeRun || !deps.sameProviderRun(activeRun, run))) {
            deps.logProviderRunDebug("session poll refreshed provider run", run, {
              session_id: session.id,
              previous_provider_run_id: activeRun?.id ?? null,
              previous_model: activeRun?.model ?? null,
              previous_variant: activeRun?.variant ?? null,
              previous_usage_tokens_total: activeRun?.usage_tokens_total ?? null,
              refresh_reason: !activeRun
                ? "missing_run"
                : activeRun.id !== session.active_provider_run_id
                  ? "run_changed"
                  : activeRun.usage_tokens_total !== run.usage_tokens_total
                    ? "usage_changed"
                    : activeRun.model !== run.model
                      ? "model_changed"
                      : activeRun.variant !== run.variant
                        ? "variant_changed"
                        : activeRun.state !== run.state
                          ? "state_changed"
                          : "metadata_changed",
            })
            deps.setProviderRun(run)
            deps.applySessionState(deps.projectSession(deps.getSession(), run))
            deps.updateSessionChrome()
          }
        } else if (deps.getProviderRun()) {
          deps.logProviderRunDebug("session poll cleared provider run", deps.getProviderRun(), {
            session_id: session.id,
          })
          deps.setProviderRun(null)
          deps.updateSessionChrome()
          if (sessionShouldRecoverMissingActiveProviderRun(session)) {
            void deps.recoverProviderRun("missing active provider run")
          }
        }
      },
    })
  }

  const pollRoomEnvironmentActivity = async () => {
    await runPollingLoop({
      ...commonOptions,
      operation: "polling Room environment activity",
      intervalMs: 200,
      task: async () => {
        await deps.synchronizeRoomEnvironmentActivity()
      },
    })
  }

  return {
    pollOutput,
    pollNotices,
    pollSessionState,
    pollRoomEnvironmentActivity,
  }
}

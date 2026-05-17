type KernelResyncAttachment = {
  id: string
}

type KernelResyncControllerOptions<TSession, TProviderRun> = {
  getAttachment: () => KernelResyncAttachment | null
  isAttached: () => boolean
  getSessionId: () => string
  getSessionStateSnapshot: () => TSession
  catchUpAttachedSession: (sessionId: string, attachmentId: string, session: TSession) => Promise<void>
  getSessionState: (sessionId: string) => Promise<TSession>
  getActiveProviderRunId: (session: TSession) => string | null
  getProviderRunState: () => TProviderRun | null
  tryGetProviderRun: (providerRunId: string) => Promise<TProviderRun | null>
  sameProviderRun: (currentRun: TProviderRun, nextRun: TProviderRun) => boolean
  projectSession: (session: TSession, providerRun: TProviderRun | null) => TSession
  shouldRefreshAgentPanesForSessionChange: (session: TSession) => boolean
  sessionHasPromptWork: (session: TSession) => boolean
  applySession: (session: TSession) => void
  applyProviderRun: (providerRun: TProviderRun | null) => void
  refreshAgentPanes: (session: TSession) => Promise<void>
  clearLocalBusyStateForAuthoritativeIdle: (session: TSession) => void
  onProviderRunCleared: (run: TProviderRun, sessionId: string, reason: string) => void
  onProviderRunRefreshed: (
    run: TProviderRun,
    sessionId: string,
    previousProviderRunId: string | null,
    reason: string,
  ) => void
  onResyncStart: (sessionId: string, attachmentId: string, reason: string) => void
  onResyncComplete: (reason: string) => void
  onResyncFailed: (reason: string, error: unknown) => void
}

export type KernelResyncController = {
  resync(reason: string): Promise<void>
  isInFlight(): boolean
}

export function createKernelResyncController<TSession, TProviderRun extends { id?: string | null }>(
  options: KernelResyncControllerOptions<TSession, TProviderRun>,
): KernelResyncController {
  let inFlight: Promise<void> | null = null

  return {
    resync(reason) {
      if (inFlight) {
        return inFlight
      }
      inFlight = (async () => {
        const attachment = options.getAttachment()
        if (!attachment || !options.isAttached()) {
          return
        }
        const sessionId = options.getSessionId()
        options.onResyncStart(sessionId, attachment.id, reason)
        await options.catchUpAttachedSession(sessionId, attachment.id, options.getSessionStateSnapshot())
        const previousSession = options.getSessionStateSnapshot()
        const nextSession = await options.getSessionState(sessionId)
        if (!options.isAttached() || options.getSessionId() !== sessionId) {
          return
        }

        const projectedSession = options.projectSession(nextSession, options.getProviderRunState())
        const shouldRefreshPanes = options.shouldRefreshAgentPanesForSessionChange(projectedSession)
        const promptJustCompleted = (
          options.sessionHasPromptWork(previousSession)
          && !options.sessionHasPromptWork(projectedSession)
        )
        options.applySession(projectedSession)

        const nextProviderRunId = options.getActiveProviderRunId(nextSession)
        if (!nextProviderRunId) {
          const activeRun = options.getProviderRunState()
          if (activeRun) {
            options.onProviderRunCleared(activeRun, sessionId, reason)
            options.applyProviderRun(null)
          }
        } else {
          const activeRun = options.getProviderRunState()
          const run = await options.tryGetProviderRun(nextProviderRunId)
          if (run && (!activeRun || !options.sameProviderRun(activeRun, run))) {
            options.onProviderRunRefreshed(
              run,
              sessionId,
              activeRun?.id ?? null,
              reason,
            )
            options.applyProviderRun(run)
            options.applySession(options.projectSession(options.getSessionStateSnapshot(), run))
          }
        }

        if (shouldRefreshPanes || promptJustCompleted || reason === "transport_resumed" || reason === "replay_gap") {
          await options.refreshAgentPanes(options.getSessionStateSnapshot())
        }
        options.clearLocalBusyStateForAuthoritativeIdle(options.getSessionStateSnapshot())
        options.onResyncComplete(reason)
      })().catch((error) => {
        options.onResyncFailed(reason, error)
      }).finally(() => {
        inFlight = null
      })
      return inFlight
    },
    isInFlight() {
      return inFlight !== null
    },
  }
}

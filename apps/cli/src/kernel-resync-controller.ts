import type {
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import {
  sessionSnapshotRefreshTransition,
} from "@chariox/kernel-client/session-runtime-transition"

const RESYNC_FORCE_PANE_REFRESH_REASONS = ["transport_resumed", "replay_gap"] as const

type KernelResyncAttachment = {
  id: string
}

type KernelResyncControllerOptions = {
  getAttachment: () => KernelResyncAttachment | null
  isAttached: () => boolean
  getSessionId: () => string
  getSessionStateSnapshot: () => RuntimeSession
  catchUpAttachedSession: (sessionId: string, attachmentId: string, session: RuntimeSession) => Promise<void>
  getSessionState: (sessionId: string) => Promise<RuntimeSession>
  getActiveProviderRunId: (session: RuntimeSession) => string | null
  getProviderRunState: () => RuntimeProviderRun | null
  tryGetProviderRun: (providerRunId: string) => Promise<RuntimeProviderRun | null>
  sameProviderRun: (currentRun: RuntimeProviderRun, nextRun: RuntimeProviderRun) => boolean
  projectSession: (session: RuntimeSession, providerRun: RuntimeProviderRun | null) => RuntimeSession
  shouldRefreshAgentPanesForSessionChange: (session: RuntimeSession) => boolean
  applySession: (session: RuntimeSession) => void
  applyProviderRun: (providerRun: RuntimeProviderRun | null) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  clearLocalBusyStateForAuthoritativeIdle: (session: RuntimeSession) => void
  onProviderRunCleared: (run: RuntimeProviderRun, sessionId: string, reason: string) => void
  onProviderRunRefreshed: (
    run: RuntimeProviderRun,
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

export function createKernelResyncController(options: KernelResyncControllerOptions): KernelResyncController {
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
        const refreshTransition = sessionSnapshotRefreshTransition({
          previousSession,
          nextSession: projectedSession,
          sessionChangeRequiresPaneRefresh:
            options.shouldRefreshAgentPanesForSessionChange(projectedSession),
          reason,
          forcePaneRefreshReasons: RESYNC_FORCE_PANE_REFRESH_REASONS,
        })
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

        if (refreshTransition.shouldRefreshAgentPanes) {
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

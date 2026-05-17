type KernelRestartRecoveryControllerOptions<TSession, TAttachment> = {
  initialDelayMs?: number
  maxDelayMs?: number
  isClosing: () => boolean
  isAttached: () => boolean
  isDisconnected: () => boolean
  getSessionId: () => string | null
  getSessionState: (sessionId: string) => Promise<TSession>
  attachToSession: (sessionId: string) => Promise<TAttachment>
  projectSession: (session: TSession) => TSession
  applyAttachment: (attachment: TAttachment) => void
  applySession: (session: TSession) => void
  resetKernelEventSubscription: () => void
  syncKernelEventSubscription: () => Promise<void>
  refreshAgentPanes: () => Promise<void>
  clearLocalBusyStateForAuthoritativeIdle: () => void
  onRecovered: () => void
  onAttemptFailed: (sessionId: string, error: unknown) => void
  sleep: (delayMs: number) => Promise<void>
}

export type KernelRestartRecoveryController = {
  recover(): Promise<void> | null
  isInFlight(): boolean
}

export function createKernelRestartRecoveryController<TSession, TAttachment>(
  options: KernelRestartRecoveryControllerOptions<TSession, TAttachment>,
): KernelRestartRecoveryController {
  const initialDelayMs = options.initialDelayMs ?? 250
  const maxDelayMs = options.maxDelayMs ?? 5_000
  let inFlight: Promise<void> | null = null

  const stillRecoveringSession = (sessionId: string) => (
    !options.isClosing()
    && options.isAttached()
    && options.getSessionId() === sessionId
    && options.isDisconnected()
  )

  return {
    recover() {
      if (inFlight) {
        return inFlight
      }

      const sessionId = options.getSessionId()
      if (!options.isAttached() || !sessionId) {
        return null
      }

      inFlight = (async () => {
        let delayMs = initialDelayMs
        while (stillRecoveringSession(sessionId)) {
          try {
            const nextSession = await options.getSessionState(sessionId)
            if (!stillRecoveringSession(sessionId)) {
              return
            }
            const nextAttachment = await options.attachToSession(sessionId)
            if (!stillRecoveringSession(sessionId)) {
              return
            }
            options.applyAttachment(nextAttachment)
            options.applySession(options.projectSession(nextSession))
            options.resetKernelEventSubscription()
            await options.syncKernelEventSubscription()
            await options.refreshAgentPanes()
            options.clearLocalBusyStateForAuthoritativeIdle()
            options.onRecovered()
            return
          } catch (error) {
            options.onAttemptFailed(sessionId, error)
            await options.sleep(delayMs)
            delayMs = Math.min(delayMs * 2, maxDelayMs)
          }
        }
      })().finally(() => {
        inFlight = null
      })
      return inFlight
    },
    isInFlight() {
      return inFlight !== null
    },
  }
}

type KernelSessionUnavailableControllerOptions<TSession extends { id: string }, TAttachment, TProviderRun> = {
  isAttached: () => boolean
  getSession: () => TSession
  getProviderRun: () => TProviderRun | null
  getSessionState: (sessionId: string) => Promise<TSession>
  attachToSession: (sessionId: string) => Promise<TAttachment>
  applyAttachment: (attachment: TAttachment) => void
  projectSession: (session: TSession, providerRun: TProviderRun | null) => TSession
  applySession: (session: TSession) => void
  resetKernelEventSubscription: () => void
  syncKernelEventSubscription: () => Promise<void>
  refreshAgentPanes: (session: TSession) => Promise<void>
  clearLocalBusyStateForAuthoritativeIdle: (session: TSession) => void
  recordDaemonActivity: (activityType: string) => void
  onRecovered: () => void
  onStateLookupFailed: (sessionId: string, message: string, error: unknown) => void
  transitionToNoSession: (message: string) => Promise<void>
}

export function createKernelSessionUnavailableController<TSession extends { id: string }, TAttachment, TProviderRun>(
  options: KernelSessionUnavailableControllerOptions<TSession, TAttachment, TProviderRun>,
) {
  const handle = async (message: string) => {
    const sessionId = options.getSession().id
    if (options.isAttached() && sessionId) {
      try {
        const nextSession = await options.getSessionState(sessionId)
        const nextAttachment = await options.attachToSession(sessionId)
        if (!options.isAttached() || options.getSession().id !== sessionId) {
          return
        }
        options.applyAttachment(nextAttachment)
        options.applySession(options.projectSession(nextSession, options.getProviderRun()))
        options.resetKernelEventSubscription()
        await options.syncKernelEventSubscription()
        const recoveredSession = options.getSession()
        await options.refreshAgentPanes(recoveredSession)
        options.clearLocalBusyStateForAuthoritativeIdle(recoveredSession)
        options.recordDaemonActivity("kernel_session_unavailable_recovered")
        options.onRecovered()
        return
      } catch (error) {
        options.onStateLookupFailed(sessionId, message, error)
      }
    }
    await options.transitionToNoSession(message)
  }

  return { handle }
}

type ProviderRecoveryLaunchInput = {
  sessionId: string
  provider: string
  accountProfile: string
  model: string
  effort: string
  targetAgentId: string | null
}

type ProviderRecoveryControllerOptions<TSession, TProviderRun> = {
  isAttached: () => boolean
  getSessionId: () => string
  getProvider: () => string
  getAccountProfile: () => string
  getModel: () => string
  getEffort: () => string
  getTargetAgentId: () => string | null
  launchProviderRun: (input: ProviderRecoveryLaunchInput) => Promise<TProviderRun>
  getSessionState: (sessionId: string) => Promise<TSession>
  projectSession: (session: TSession, providerRun: TProviderRun) => TSession
  applyProviderRun: (providerRun: TProviderRun) => void
  applySession: (session: TSession) => void
  resizeSession: (sessionId: string) => Promise<void>
  onRecovered: (reason: string) => void
  onRecoveryFailed: (reason: string, error: unknown) => void
}

export type ProviderRecoveryController = {
  recover(reason: string): Promise<boolean>
  isInFlight(): boolean
}

export function createProviderRecoveryController<TSession, TProviderRun>(
  options: ProviderRecoveryControllerOptions<TSession, TProviderRun>,
): ProviderRecoveryController {
  let inFlight = false

  return {
    async recover(reason) {
      if (!options.isAttached() || inFlight) {
        return false
      }

      inFlight = true
      try {
        const providerRun = await options.launchProviderRun({
          sessionId: options.getSessionId(),
          provider: options.getProvider(),
          accountProfile: options.getAccountProfile(),
          model: options.getModel(),
          effort: options.getEffort(),
          targetAgentId: options.getTargetAgentId(),
        })
        options.applyProviderRun(providerRun)
        options.applySession(
          options.projectSession(await options.getSessionState(options.getSessionId()), providerRun),
        )
        await options.resizeSession(options.getSessionId())
        options.onRecovered(reason)
        return true
      } catch (error) {
        options.onRecoveryFailed(reason, error)
        return false
      } finally {
        inFlight = false
      }
    },
    isInFlight() {
      return inFlight
    },
  }
}

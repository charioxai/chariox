import type { RuntimeSession } from "./cli-types.js"
import {
  resolvePromptRecoveryProviderLaunch,
  type SessionLifecycleLaunchSelection,
} from "@chariox/kernel-client/session-lifecycle-state"

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
  getSessionStateSnapshot: () => RuntimeSession
  getFallbackLaunch: () => SessionLifecycleLaunchSelection
  getAccountProfile: () => string
  getTargetAgentId: () => string | null
  launchProviderRun: (input: ProviderRecoveryLaunchInput) => Promise<TProviderRun>
  getSessionState: (sessionId: string) => Promise<TSession>
  projectSession: (session: TSession, providerRun: TProviderRun) => TSession
  applyProviderRun: (providerRun: TProviderRun) => void
  applySession: (session: TSession) => void
  resizeSession: (sessionId: string) => Promise<void>
  onRecovered: (reason: string) => void
  onRecoverySkipped: (reason: string, skipReason: string) => void
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
        const launchDecision = resolvePromptRecoveryProviderLaunch(
          options.getSessionStateSnapshot(),
          options.getFallbackLaunch(),
          options.getTargetAgentId(),
        )
        if (launchDecision.action === "skip_launch") {
          options.onRecoverySkipped(reason, launchDecision.reason)
          return false
        }
        const providerRun = await options.launchProviderRun({
          sessionId: options.getSessionId(),
          provider: launchDecision.launch.provider,
          accountProfile: options.getAccountProfile(),
          model: launchDecision.launch.model,
          effort: launchDecision.launch.effort,
          targetAgentId: launchDecision.targetAgentId,
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

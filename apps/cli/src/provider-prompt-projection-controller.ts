import type {
  AgentInstance,
  RuntimeProviderRun,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type {
  PromptMetaPart,
  PromptUsageMeta,
  PromptMetaStateOptions,
} from "@chariox/kernel-client/prompt-meta"
import {
  derivePromptMetaState,
  derivePromptUsageState,
} from "@chariox/kernel-client/prompt-meta"
import {
  derivePromptProviderSelection,
} from "@chariox/kernel-client/prompt-provider-selection"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type ProviderPromptProjectionDefaults = {
  provider?: string
  accountProfile?: string
  model: string
  effort: string
}

export type ProviderPromptProjectionControllerDeps = {
  getProviderRun: () => RuntimeProviderRun | null
  getFocusedAgent: () => AgentInstance | null
  getWaitingRoomState: () => WaitingRoomState
  getDefaults: () => ProviderPromptProjectionDefaults
  getProviderCatalog: () => ProviderCatalog
}

export function createProviderPromptProjectionController(
  deps: ProviderPromptProjectionControllerDeps,
) {
  const providerSelectionOptions = (): PromptMetaStateOptions => {
    const defaults = deps.getDefaults()
    const options = {
      providerRun: deps.getProviderRun(),
      focusedAgent: deps.getFocusedAgent(),
      waitingRoomState: deps.getWaitingRoomState(),
      defaultModel: defaults.model,
      defaultEffort: defaults.effort,
    }
    return defaults.provider
      ? { ...options, defaultProvider: defaults.provider }
      : options
  }

  // The CLI's cycling waiting room selector always highlights a concrete
  // provider/model/effort (there is no blank dropdown state), so this
  // adapter coerces the shared, nullable selection back to the CLI's
  // sentinel-based defaults at the boundary. `deps.getDefaults()` is
  // sourced from `config.toml` (falling back to "opencode" only when
  // nothing has ever been configured), so this still prefers the
  // persisted default over a hardcoded literal.
  const currentProviderSelection = (): {
    provider: string
    accountProfile: string
    model: string
    effort: string
  } => {
    const selection = derivePromptProviderSelection(providerSelectionOptions())
    const defaults = deps.getDefaults()
    return {
      provider: selection.provider ?? defaults.provider ?? "opencode",
      accountProfile: deps.getProviderRun()?.account_profile
        ?? deps.getFocusedAgent()?.account_profile
        ?? deps.getWaitingRoomState().accountProfileId
        ?? defaults.accountProfile
        ?? "default",
      model: selection.model ?? defaults.model,
      effort: selection.effort ?? defaults.effort,
    }
  }

  return {
    currentProviderSelection,

    promptMetaParts(): PromptMetaPart[] {
      return derivePromptMetaState(providerSelectionOptions())
    },

    promptUsageMeta(): PromptUsageMeta | null {
      return derivePromptUsageState({
        providerRun: deps.getProviderRun(),
        focusedAgent: deps.getFocusedAgent(),
        catalog: deps.getProviderCatalog(),
      })
    },

    currentModelId(): string {
      return currentProviderSelection().model
    },

    currentVariantId(): string {
      return currentProviderSelection().effort
    },
  }
}

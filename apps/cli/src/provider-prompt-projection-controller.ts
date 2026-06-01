import type {
  AgentInstance,
  RuntimeProviderRun,
} from "./cli-types.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type {
  PromptMetaPart,
  PromptUsageMeta,
} from "./prompt-meta.js"
import {
  deriveCurrentProviderSelection,
  derivePromptMetaState,
  derivePromptUsageState,
} from "./session-chrome-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type ProviderPromptProjectionDefaults = {
  provider?: string
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
  const providerSelectionOptions = () => {
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

  const currentProviderSelection = () => deriveCurrentProviderSelection(providerSelectionOptions())

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

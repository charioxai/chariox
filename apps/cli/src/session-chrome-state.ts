import type { AgentInstance, RuntimeProviderRun } from "./cli-types.js"
import {
  derivePromptProviderSelection,
  providerRunForPromptSelection,
  resolveProviderModelContextLimit,
  type PromptProviderSelectionOptions,
} from "@arroba/kernel-client/prompt-provider-selection"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  formatPromptMetaParts,
  formatPromptUsageMeta,
  type PromptMetaPart,
  type PromptUsageMeta,
} from "./prompt-meta.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

type ProviderSelectionOptions = {
  providerRun: RuntimeProviderRun | null
  focusedAgent?: AgentInstance | null
  waitingRoomState: WaitingRoomState
  defaultProvider?: string
  defaultModel: string
  defaultEffort: string
}

export function derivePromptMetaState(options: ProviderSelectionOptions): PromptMetaPart[] {
  const selection = derivePromptProviderSelection(options as PromptProviderSelectionOptions)
  return formatPromptMetaParts(
    selection.provider,
    selection.model,
    selection.effort,
  )
}

export function derivePromptUsageState(options: {
  providerRun: RuntimeProviderRun | null
  focusedAgent?: AgentInstance | null
  catalog: ProviderCatalog
}): PromptUsageMeta | null {
  const run = providerRunForPromptSelection(options.providerRun, options.focusedAgent)
  if (!run) {
    return null
  }

  return formatPromptUsageMeta(
    run.usage_tokens_total,
    run.usage?.context_tokens,
    resolveProviderModelContextLimit(options.catalog, run.provider, run.model),
    12,
  )
}

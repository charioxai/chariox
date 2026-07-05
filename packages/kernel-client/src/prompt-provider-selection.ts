import type { AgentInstance, RuntimeProviderRun, RuntimeSession } from "./kernel-types.js"

export type PromptProviderSelectionWaitingRoomState = {
  readonly providerId?: string | null
  readonly modelId?: string | null
  readonly effort?: string | null
}

export type PromptProviderSelectionOptions = {
  readonly providerRun: RuntimeProviderRun | null
  readonly focusedAgent?: AgentInstance | null
  readonly waitingRoomState: PromptProviderSelectionWaitingRoomState
  readonly defaultProvider?: string | null
  readonly defaultModel: string
  readonly defaultEffort: string
}

export type PromptProviderSelection = {
  readonly provider: string
  readonly model: string
  readonly effort: string
}

export type ProviderModelContextCatalog = {
  readonly all: readonly ProviderModelContextProvider[]
}

export type ProviderModelContextProvider = {
  readonly id: string
  readonly models: Record<string, ProviderModelContextModel | undefined>
}

export type ProviderModelContextModel = {
  readonly limit?: {
    readonly context?: number | null
  } | null
}

export function derivePromptProviderSelection(
  options: PromptProviderSelectionOptions,
): PromptProviderSelection {
  const providerRun = providerRunForPromptSelection(options.providerRun, options.focusedAgent)
  return {
    provider: providerRun?.provider
      ?? normalizePromptProvider(options.focusedAgent?.provider)
      ?? options.waitingRoomState.providerId
      ?? options.defaultProvider
      ?? "opencode",
    model: providerRun?.model
      ?? options.focusedAgent?.model
      ?? options.waitingRoomState.modelId
      ?? options.defaultModel,
    effort: providerRun?.variant
      ?? options.focusedAgent?.effort
      ?? options.waitingRoomState.effort
      ?? options.defaultEffort,
  }
}

export function providerRunForPromptSelection(
  providerRun: RuntimeProviderRun | null,
  focusedAgent: AgentInstance | null | undefined,
): RuntimeProviderRun | null {
  if (!providerRun || !focusedAgent) {
    return providerRun
  }
  return providerRun.agent_instance_id === focusedAgent.id ? providerRun : null
}

export function applyProviderRunProfileToSession(
  session: RuntimeSession,
  providerRun: RuntimeProviderRun | null,
): RuntimeSession {
  const agentId = providerRun?.agent_instance_id
  if (!agentId) {
    return session
  }

  let changed = false
  const agents = session.agents.map((agent) => {
    if (agent.id !== agentId) {
      return agent
    }
    if (
      agent.provider === providerRun.provider
      && agent.model === providerRun.model
      && agent.effort === providerRun.variant
    ) {
      return agent
    }
    changed = true
    return {
      ...agent,
      provider: providerRun.provider,
      model: providerRun.model,
      effort: providerRun.variant,
    }
  })

  return changed ? { ...session, agents } : session
}

export function resolveProviderModelContextLimit(
  catalog: ProviderModelContextCatalog,
  providerId: string,
  modelRef: string,
): number | null {
  const normalizedModelRef = modelRef.trim()
  const parsed = normalizedModelRef.includes("/")
    ? splitProviderModelRef(normalizedModelRef)
    : { providerId, modelId: normalizedModelRef }
  if (!parsed) {
    return null
  }

  return catalog.all.find((item) => item.id === parsed.providerId)?.models[parsed.modelId]?.limit?.context ?? null
}

export function splitProviderModelRef(modelRef: string): { providerId: string; modelId: string } | null {
  const parts = modelRef.split("/").filter(Boolean)
  if (parts.length < 2) {
    return null
  }
  return {
    providerId: parts.at(-2)!,
    modelId: parts.at(-1)!,
  }
}

export function normalizePromptProvider(provider?: string | null): string | null {
  if (!provider || provider === "default") {
    return null
  }
  return provider
}

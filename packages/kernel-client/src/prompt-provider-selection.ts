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
  readonly defaultModel?: string | null
  readonly defaultEffort?: string | null
}

/**
 * `null` means nothing is selected yet: no configured `config.toml` default,
 * no waiting room selection, and no active provider run/agent to inherit
 * from. Callers must treat a `null` field as "a session cannot be started."
 */
export type PromptProviderSelection = {
  readonly provider: string | null
  readonly model: string | null
  readonly effort: string | null
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
      ?? null,
    model: providerRun?.model
      ?? options.focusedAgent?.model
      ?? options.waitingRoomState.modelId
      ?? options.defaultModel
      ?? null,
    effort: providerRun?.variant
      ?? options.focusedAgent?.effort
      ?? options.waitingRoomState.effort
      ?? options.defaultEffort
      ?? null,
  }
}

export function providerRunForPromptSelection(
  providerRun: RuntimeProviderRun | null,
  focusedAgent: AgentInstance | null | undefined,
): RuntimeProviderRun | null {
  if (!providerRun || !focusedAgent) {
    return providerRun
  }
  return providerRun.agent_instance_id === focusedAgent.id && runDescribesAgent(providerRun, focusedAgent)
    ? providerRun
    : null
}

export function applyProviderRunProfileToSession(
  session: RuntimeSession,
  providerRun: RuntimeProviderRun | null,
): RuntimeSession {
  const agentId = providerRun?.agent_instance_id
  // A cached run is observational state. Only the kernel-selected current run
  // may refine an agent profile; a replaced or cleared run cannot undo a switch.
  if (!agentId || providerRun.id !== session.active_provider_run_id) {
    return session
  }

  let changed = false
  const agents = session.agents.map((agent) => {
    if (agent.id !== agentId || !runDescribesAgent(providerRun, agent)) {
      return agent
    }
    if (
      agent.model === providerRun.model
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

// OpenCode runs report their backend provider (e.g. "openai").
const harnessIds = new Set(["opencode", "codex", "claude-headless", "claude-p"])
// Only the kernel commits a provider switch: a run describes its agent when it
// runs the agent's provider or one of its turn substitutes, so a lagging run of
// the previous provider cannot revert the switch.
function runDescribesAgent(run: RuntimeProviderRun, agent: AgentInstance): boolean {
  return sameHarness(run.provider, agent.provider)
    || (agent.substitutes ?? []).some((substitute) =>
      sameHarness(substitute.provider, run.provider)
      && substitute.model === run.model
      && (!substitute.account_profile || substitute.account_profile === run.account_profile))
}

function sameHarness(left: string, right: string): boolean {
  const harness = (id: string) => id === "claude" ? "claude-p" : harnessIds.has(id) ? id : "opencode"
  return harness(left) === harness(right)
}

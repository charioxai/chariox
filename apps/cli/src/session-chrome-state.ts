import type { AgentInstance, RuntimeProviderRun, RuntimeSession, WorkspaceLiveSyncStatus } from "./cli-types.js"
import {
  sessionAttachedFooterSummary,
  sessionFooterHint,
  sessionFocusedStatusBadge,
  sessionStatusMode,
  type SessionAgentBusyState,
  type SessionFocusedStatusBadge,
  type SessionStatusMode as KernelSessionStatusMode,
  type SessionStatusBadgePart,
} from "@arroba/kernel-client/shell-agent-activity"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  formatPromptMetaParts,
  formatPromptUsageMeta,
  type PromptMetaPart,
  type PromptUsageMeta,
} from "./prompt-meta.js"
import { chooseVisibleActivityLabel } from "./runtime.js"
import { agentPaneStatusBadge, type SplitPaneFooterAgent } from "./split-pane-footer.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type SessionStatusMode = KernelSessionStatusMode
export type StatusBadgePart = SessionStatusBadgePart
export type FocusedStatusBadge = SessionFocusedStatusBadge

type ProviderSelectionOptions = {
  providerRun: RuntimeProviderRun | null
  focusedAgent?: AgentInstance | null
  waitingRoomState: WaitingRoomState
  defaultProvider?: string
  defaultModel: string
  defaultEffort: string
}

export function deriveCurrentProviderSelection(options: ProviderSelectionOptions) {
  const providerRun = providerRunForFocusedSelection(options.providerRun, options.focusedAgent)
  return {
    provider: providerRun?.provider
      ?? normalizeProvider(options.focusedAgent?.provider)
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

function providerRunForFocusedSelection(
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

export function derivePromptMetaState(options: ProviderSelectionOptions): PromptMetaPart[] {
  const selection = deriveCurrentProviderSelection(options)
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
  const run = providerRunForFocusedSelection(options.providerRun, options.focusedAgent)
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

export function deriveSessionStatusMode(options: {
  daemonDisconnected: boolean
  working: boolean
  hasActivePrompt: boolean
  submitting: boolean
  queueDepth: number
}): SessionStatusMode {
  return sessionStatusMode(options)
}

export function deriveFooterHint(options: {
  fatalError: string | null
  activePromptId: string | null
  queueDepth: number
  statusLine: string
}): string {
  return sessionFooterHint(options)
}

export function deriveVisibleActivityLabel(options: {
  providerActivityLabel: string | null
  activeToolLabels: Iterable<string>
}) {
  const latestActiveToolLabel = Array.from(options.activeToolLabels).at(-1) ?? null
  return chooseVisibleActivityLabel(options.providerActivityLabel, latestActiveToolLabel)
}

export type AgentBusyState = SessionAgentBusyState

export function deriveFocusedStatusBadge(options: {
  attached: boolean
  daemonDisconnected: boolean
  activeStatusLabel: string | null
  focusedBusy: boolean
  agents?: AgentBusyState[]
}): FocusedStatusBadge {
  return sessionFocusedStatusBadge(options)
}

export function deriveAttachedFooterSummary(options: {
  session: RuntimeSession
  connectedClientCount: number
  multiAgentMode: boolean
  responseLayout: MultiAgentResponseLayout
  sessionStatusMode: SessionStatusMode
  hotkeyToggleLabel: string
  focusedHasPromptWork?: boolean
  workspaceLiveSyncStatus?: WorkspaceLiveSyncStatus | null
}): string {
  return sessionAttachedFooterSummary(options)
}

function resolveProviderModelContextLimit(
  catalog: ProviderCatalog,
  providerId: string,
  modelRef: string,
) {
  const normalizedModelRef = modelRef.trim()
  const parsed = normalizedModelRef.includes("/")
    ? splitProviderModelRef(normalizedModelRef)
    : { providerId, modelId: normalizedModelRef }
  if (!parsed) {
    return null
  }

  return catalog.all.find((item) => item.id === parsed.providerId)?.models[parsed.modelId]?.limit?.context ?? null
}

function splitProviderModelRef(modelRef: string) {
  const parts = modelRef.split("/").filter(Boolean)
  if (parts.length < 2) {
    return null
  }
  return {
    providerId: parts.at(-2)!,
    modelId: parts.at(-1)!,
  }
}

function normalizeProvider(provider?: string | null) {
  if (!provider || provider === "default") {
    return null
  }
  return provider
}

import type { ProviderCatalog } from "./provider-catalog.js"
import { getSessionStatusLabel } from "./runtime.js"

export type StatusBadgeTone = "idle" | "working" | "disconnected" | "error"

export type SplitPaneFooterMode = "idle" | "working" | "disconnected"

export type SplitPaneFooterAgent = {
  id: string
  agent_ref: string
  alias: string | null
  provider: string
  model: string | null
  state: "Idle" | "Working" | "Focused" | "Error"
  is_processing: boolean
}

export type SplitPaneFooterActiveRun = {
  agentInstanceId: string | null
  model: string | null
}

export type SplitPaneFooterPaneState = {
  badge: {
    label: string
    tone: StatusBadgeTone
  }
  focused: boolean
  info: string
}

export type SplitPaneFooterState<T extends SplitPaneFooterAgent> = {
  primary: SplitPaneFooterPaneState
  secondary: SplitPaneFooterPaneState
  tertiary: SplitPaneFooterPaneState
  selection: {
    primary: T | null
    secondary: T | null
    tertiary: T | null
  }
}

export function reflectedDistance(index: number, length: number, frame: number): number {
  if (length <= 1) {
    return 0
  }

  const span = length - 1
  const cycle = span * 2
  const position = frame % cycle
  const highlight = position <= span ? position : cycle - position
  return Math.abs(index - highlight)
}

export function agentPaneStatusBadge(
  agent: SplitPaneFooterAgent | null,
  activeLabel: string | null,
  isStreaming = false,
) {
  if (!agent) {
    return { label: "", tone: "idle" as const }
  }
  if (agent.state === "Error") {
    return { label: "ERROR", tone: "error" as const }
  }
  if (activeLabel) {
    return { label: getSessionStatusLabel("working", activeLabel), tone: "working" as const }
  }
  if (agent.is_processing || agent.state === "Working" || isStreaming) {
    return { label: "WORKING", tone: "working" as const }
  }
  return { label: "IDLE", tone: "idle" as const }
}

export function resolveAgentModelLabel(
  catalog: ProviderCatalog,
  agent: SplitPaneFooterAgent | null,
  activeRun?: SplitPaneFooterActiveRun | null,
  fallbackModel?: string | null,
) {
  if (!agent) {
    return "No model"
  }

  const effectiveModel = activeRun?.model && activeRun.agentInstanceId === agent.id
    ? activeRun.model
    : agent.model ?? fallbackModel ?? null
  const normalizedModel = effectiveModel?.trim() ?? ""
  if (!normalizedModel || normalizedModel === "default") {
    return "Default model"
  }

  const parsed = splitProviderModelRef(normalizedModel)
  const provider = catalog.all.find((entry) => entry.id === (parsed?.providerId ?? agent.provider))
  const model = provider?.models?.[parsed?.modelId ?? normalizedModel]
  return model?.name ?? normalizedModel
}

export function formatSplitPaneFooter(
  agent: SplitPaneFooterAgent | null,
  catalog: ProviderCatalog,
  activeRun?: SplitPaneFooterActiveRun | null,
  fallbackModel?: string | null,
) {
  if (!agent) {
    return ""
  }
  const aliasLabel = agent.alias?.trim() || agent.agent_ref
  const modelLabel = resolveAgentModelLabel(catalog, agent, activeRun, fallbackModel)
  return `${aliasLabel} • ${modelLabel}`
}

export function buildSplitPaneFooterState<T extends SplitPaneFooterAgent>(options: {
  mode: SplitPaneFooterMode
  selection: {
    primary: T | null
    secondary: T | null
    tertiary: T | null
  }
  focusedAgentId: string | null
  streamingAgentId: string | null
  activityLabels: Record<string, string | null>
  catalog: ProviderCatalog
  activeRun?: SplitPaneFooterActiveRun | null
  fallbackModel?: string | null
}): SplitPaneFooterState<T> {
  const buildPaneState = (agent: T | null): SplitPaneFooterPaneState => {
    const focused = agent?.id === options.focusedAgentId
    const badge = options.mode === "disconnected"
      ? { label: "DISCONNECTED", tone: "disconnected" as const }
      : agentPaneStatusBadge(
        agent,
        agent ? options.activityLabels[agent.id] ?? null : null,
        agent?.id === options.streamingAgentId,
      )
    return {
      badge,
      focused,
      info: formatSplitPaneFooter(agent, options.catalog, options.activeRun, options.fallbackModel),
    }
  }

  return {
    primary: buildPaneState(options.selection.primary),
    secondary: buildPaneState(options.selection.secondary),
    tertiary: buildPaneState(options.selection.tertiary),
    selection: options.selection,
  }
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

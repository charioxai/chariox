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
  fallbackModel?: string | null,
) {
  if (!agent) {
    return "No model"
  }
  const effectiveModel = agent.model ?? fallbackModel ?? null
  const provider = catalog.all.find((entry) => entry.id === agent.provider)
  const model = effectiveModel ? provider?.models?.[effectiveModel] : null
  return model?.name ?? effectiveModel ?? "Default model"
}

export function formatSplitPaneFooter(
  agent: SplitPaneFooterAgent | null,
  catalog: ProviderCatalog,
  fallbackModel?: string | null,
) {
  if (!agent) {
    return ""
  }
  const aliasLabel = agent.alias?.trim() || agent.agent_ref
  const modelLabel = resolveAgentModelLabel(catalog, agent, fallbackModel)
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
      info: formatSplitPaneFooter(agent, options.catalog, options.fallbackModel),
    }
  }

  return {
    primary: buildPaneState(options.selection.primary),
    secondary: buildPaneState(options.selection.secondary),
    tertiary: buildPaneState(options.selection.tertiary),
    selection: options.selection,
  }
}

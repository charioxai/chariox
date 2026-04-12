import { getSessionStatusLabel } from "./runtime.js"
import { formatPromptMetaParts, type PromptMetaPart, type PromptMetaTone } from "./prompt-meta.js"

export type StatusBadgeTone = "idle" | "working" | "disconnected" | "error"

export type SplitPaneFooterMode = "idle" | "working" | "disconnected"

export type SplitPaneFooterAgent = {
  id: string
  agent_ref: string
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  state: "Idle" | "Working" | "Focused" | "Error"
  is_processing: boolean
}

export type SplitPaneFooterActiveRun = {
  agentInstanceId: string | null
  model: string | null
  variant: string | null
}

export type SplitPaneFooterOverride = {
  model?: string | null
  variant?: string | null
}

export type SplitPaneFooterPart = PromptMetaPart | {
  kind: "agent"
  text: string
  tone: PromptMetaTone
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
  hasPromptWork = false,
  isStreaming = false,
  busyLatch = false,
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
  if (hasPromptWork || agent.is_processing || agent.state === "Working" || isStreaming || busyLatch) {
    return { label: getSessionStatusLabel("working", null), tone: "working" as const }
  }
  return { label: "IDLE", tone: "idle" as const }
}

export function formatSplitPaneFooter(
  agent: SplitPaneFooterAgent | null,
  activeRun?: SplitPaneFooterActiveRun | null,
  fallbackModel?: string | null,
  override?: SplitPaneFooterOverride,
) {
  return formatSplitPaneFooterParts(agent, activeRun, fallbackModel, override)
    .map((part) => part.text)
    .join(" • ")
}

export function formatSplitPaneFooterParts(
  agent: SplitPaneFooterAgent | null,
  activeRun?: SplitPaneFooterActiveRun | null,
  fallbackModel?: string | null,
  override?: SplitPaneFooterOverride,
): SplitPaneFooterPart[] {
  if (!agent) {
    return []
  }

  const aliasLabel = agent.alias?.trim() || agent.agent_ref
  const hasActiveRun = activeRun?.agentInstanceId === agent.id
  const effectiveModel = hasActiveRun
    ? activeRun.model
    : override?.model ?? agent.model ?? fallbackModel ?? "default"
  const effectiveVariant = hasActiveRun
    ? activeRun.variant ?? agent.effort ?? override?.variant ?? ""
    : override?.variant ?? agent.effort ?? ""
  const metaRef = splitProviderModelRef(effectiveModel ?? "default")
  const provider = metaRef?.providerId ?? agent.provider
  const model = metaRef?.modelId ?? effectiveModel ?? "default"

  return [
    {
      kind: "agent",
      text: aliasLabel,
      tone: toneForAgent(aliasLabel),
    },
    ...formatPromptMetaParts(provider, model, effectiveVariant ?? ""),
  ]
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
  hasPromptWorkByAgent?: Record<string, boolean>
  busyLatchesByAgent?: Record<string, boolean>
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
        agent ? options.hasPromptWorkByAgent?.[agent.id] ?? false : false,
        agent?.id === options.streamingAgentId,
        agent ? options.busyLatchesByAgent?.[agent.id] ?? false : false,
      )
    return {
      badge,
      focused,
      info: formatSplitPaneFooter(agent, options.activeRun, options.fallbackModel),
    }
  }

  return {
    primary: buildPaneState(options.selection.primary),
    secondary: buildPaneState(options.selection.secondary),
    tertiary: buildPaneState(options.selection.tertiary),
    selection: options.selection,
  }
}

function toneForAgent(value: string): PromptMetaTone {
  const normalized = value.trim().toLowerCase()
  if (!normalized) {
    return "text"
  }
  const tones: PromptMetaTone[] = ["primary", "secondary", "accent", "warning", "success", "info"]
  let hash = 0
  for (let index = 0; index < normalized.length; index += 1) {
    hash = (hash * 31 + normalized.charCodeAt(index)) >>> 0
  }
  return tones[hash % tones.length] ?? "text"
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

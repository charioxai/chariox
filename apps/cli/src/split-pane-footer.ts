import {
  sessionAgentPaneStatusBadge,
  type SessionStatusBadgeTone,
} from "@arroba/kernel-client/session-runtime-status"
import {
  formatPromptMetaParts,
  type PromptMetaPart,
  type PromptMetaTone,
} from "@arroba/kernel-client/prompt-meta"

export type StatusBadgeTone = SessionStatusBadgeTone

export type SplitPaneFooterMode = "idle" | "working" | "disconnected"

export type SplitPaneFooterAgent = {
  id: string
  agent_ref: string
  role?: "standard" | "meta" | string
  meta_mode?: { activated_at_ms?: number; task_id?: string | null } | null
  alias: string | null
  provider: string
  model: string | null
  effort?: string | null
  substitutes?: Array<{ provider: string; model: string; variant?: string | null; kernel_id?: string | null; worktree_id?: string | null }>
  active_substitute_index?: number | null
  last_substitution?: { reason: string } | null
  execution_mode?: "build" | "plan" | null
  permission_level?: "required" | "yolo" | null
  location_label?: string | null
  state: "Idle" | "Working" | "Focused" | "Error"
  is_processing: boolean
}

export type SplitPaneFooterActiveRun = {
  agentInstanceId: string | null
  provider?: string | null
  model: string | null
  variant: string | null
}

export type SplitPaneFooterOverride = {
  provider?: string | null
  model?: string | null
  variant?: string | null
}

export type SplitPaneFooterPart = PromptMetaPart | {
  kind: "agent"
  text: string
  tone: PromptMetaTone
} | {
  kind: "mode" | "permission" | "location" | "role" | "substitute"
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

  const aliasLabel = agent.alias?.trim() || "agent"
  const provider = nonBlank(activeRun?.provider)
    ?? nonBlank(override?.provider)
    ?? nonBlank(agent.provider)
    ?? "opencode"
  const model = nonBlank(activeRun?.model)
    ?? nonBlank(override?.model)
    ?? nonBlank(agent.model)
    ?? nonBlank(fallbackModel)
    ?? "default"
  const variant = nonBlank(activeRun?.variant)
    ?? nonBlank(override?.variant)
    ?? nonBlank(agent.effort)
    ?? ""
  const runtimeLocationPart = footerLocationPart(agent.location_label)
  const substitutePart = footerSubstitutePart(agent)
  const mode = nonBlank(agent.execution_mode)
  const permission = nonBlank(agent.permission_level)
  return compactParts([
    {
      kind: "agent",
      text: aliasLabel,
      tone: toneForAgent(aliasLabel),
    },
    agent.meta_mode ? { kind: "role" as const, text: "Meta mode", tone: "accent" as const } : null,
    runtimeLocationPart,
    ...formatPromptMetaParts(provider, model, variant),
    mode ? { kind: "mode" as const, text: mode, tone: "info" as const } : null,
    permission ? { kind: "permission" as const, text: permission, tone: permission === "required" ? "warning" as const : "success" as const } : null,
    substitutePart,
  ])
}

function footerLocationPart(locationLabel: string | null | undefined): SplitPaneFooterPart | null {
  const location = nonBlank(locationLabel)
  if (!location) {
    return null
  }
  if (location.toLowerCase().startsWith("slice")) {
    return { kind: "location", text: "view slice", tone: "accent" }
  }
  return { kind: "location", text: location, tone: "accent" }
}

function footerSubstitutePart(agent: SplitPaneFooterAgent): SplitPaneFooterPart | null {
  const substitutes = agent.substitutes ?? []
  if (substitutes.length === 0 && !agent.last_substitution) {
    return null
  }
  const activeIndex = agent.active_substitute_index
  if (typeof activeIndex === "number" && activeIndex >= 0 && substitutes[activeIndex]) {
    const active = substitutes[activeIndex]
    const label = [
      nonBlank(active.provider),
      nonBlank(active.model),
      nonBlank(active.variant),
    ].filter(Boolean).join("/")
    return {
      kind: "substitute",
      text: label ? `sub ${activeIndex + 1}: ${label}` : `sub ${activeIndex + 1}`,
      tone: "warning",
    }
  }
  return {
    kind: "substitute",
    text: `${substitutes.length} sub${substitutes.length === 1 ? "" : "s"}`,
    tone: "text",
  }
}

function compactParts(parts: readonly (SplitPaneFooterPart | null | undefined)[]): SplitPaneFooterPart[] {
  return parts.filter((part): part is SplitPaneFooterPart => Boolean(part?.text.trim()))
}

function nonBlank(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
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
  useLegacyAgentProcessingState?: boolean
  activeRun?: SplitPaneFooterActiveRun | null
  fallbackModel?: string | null
}): SplitPaneFooterState<T> {
  const buildPaneState = (agent: T | null): SplitPaneFooterPaneState => {
    const focused = agent?.id === options.focusedAgentId
    const badge = options.mode === "disconnected"
      ? { label: "DISCONNECTED", tone: "disconnected" as const }
      : sessionAgentPaneStatusBadge({
        agent,
        activeLabel: agent ? options.activityLabels[agent.id] ?? null : null,
        hasPromptWork: agent ? options.hasPromptWorkByAgent?.[agent.id] ?? false : false,
        isStreaming: agent?.id === options.streamingAgentId,
        busyLatch: agent ? options.busyLatchesByAgent?.[agent.id] ?? false : false,
        useLegacyAgentProcessingState: options.useLegacyAgentProcessingState ?? true,
      })
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

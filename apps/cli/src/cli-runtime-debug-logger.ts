import type {
  RuntimeProviderRun,
  TranscriptEntry,
} from "./cli-types.js"
import type { FocusedStatusBadge } from "./session-chrome-state.js"

type RuntimeDebugLogSink = {
  debug?: (message: string, fields?: Record<string, unknown>) => void
  info?: (message: string, fields?: Record<string, unknown>) => void
}

export type CliRuntimeDebugLoggerDeps = {
  logger: RuntimeDebugLogSink | null | undefined
  debugLogsEnabled: boolean
  getResponseLayout: () => string
  splitAgentResponseMode: () => boolean
  isAttached: () => boolean
  getAgentCount: () => number
  getFocusedAgentId: () => string | null
  hasTranscriptScrollbox: () => boolean
  getVisibleTranscriptAgentId: () => string | null
}

export function createCliRuntimeDebugLogger(deps: CliRuntimeDebugLoggerDeps) {
  let lastFocusedBadgeState: string | null = null

  const logProviderRun = (
    message: string,
    run: RuntimeProviderRun | null,
    fields: Record<string, unknown> = {},
  ) => {
    deps.logger?.debug?.(message, {
      provider_run_id: run?.id ?? null,
      provider: run?.provider ?? null,
      provider_model: run?.model ?? null,
      provider_variant: run?.variant ?? null,
      provider_usage_tokens_total: run?.usage_tokens_total ?? null,
      provider_state: run?.state ?? null,
      ...fields,
    })
  }

  const logView = (phase: string, fields: Record<string, unknown> = {}) => {
    if (!deps.debugLogsEnabled) {
      return
    }
    const layout = deps.getResponseLayout()
    deps.logger?.debug?.(`view debug: ${phase}`, {
      layout,
      layout_is_split: layout === "split",
      split_active: deps.splitAgentResponseMode(),
      attached: deps.isAttached(),
      center_mode: "transcript",
      agent_count: deps.getAgentCount(),
      focused_agent_id: deps.getFocusedAgentId(),
      has_transcript_scrollbox: deps.hasTranscriptScrollbox(),
      ...fields,
    })
  }

  const logVisibleTranscriptOutput = (
    role: TranscriptEntry["role"],
    text: string,
    merged: boolean,
    mergeKey?: string,
  ) => {
    if (!["assistant", "reasoning", "tool", "error", "status"].includes(role)) {
      return
    }
    deps.logger?.info?.("applied visible transcript output", {
      role,
      merged,
      merge_key: mergeKey ?? null,
      focused_agent_id: deps.getFocusedAgentId(),
      visible_agent_id: deps.getVisibleTranscriptAgentId(),
      preview: text.replace(/\s+/g, " ").trim().slice(0, 160),
    })
  }

  const logFocusedBadgeChange = (badge: FocusedStatusBadge) => {
    const nextState = `${badge.label}:${badge.parts.map((part) => `${part.label}:${part.tone}`).join("|")}`
    if (lastFocusedBadgeState === nextState) {
      return
    }
    lastFocusedBadgeState = nextState
    deps.logger?.info?.("focused status badge changed", {
      label: badge.label,
      tone: badge.tone,
      parts: badge.parts,
      focused_agent_id: deps.getFocusedAgentId(),
      visible_agent_id: deps.getVisibleTranscriptAgentId(),
    })
  }

  const resetFocusedBadgeChange = () => {
    lastFocusedBadgeState = null
  }

  return {
    logProviderRun,
    logView,
    logVisibleTranscriptOutput,
    logFocusedBadgeChange,
    resetFocusedBadgeChange,
  }
}

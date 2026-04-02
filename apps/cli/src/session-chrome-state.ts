import type { RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  formatPromptMetaParts,
  formatPromptUsageMeta,
  type PromptMetaPart,
  type PromptUsageMeta,
} from "./prompt-meta.js"
import { chooseVisibleActivityLabel } from "./runtime.js"
import type { StatusBadgeTone } from "./split-pane-footer.js"
import { agentPaneStatusBadge, type SplitPaneFooterAgent } from "./split-pane-footer.js"
import type { WaitingRoomState } from "./waiting-room.js"

export type SessionStatusMode = "idle" | "working" | "disconnected"
export type FocusedStatusBadge = {
  label: string
  tone: StatusBadgeTone
}

type ProviderSelectionOptions = {
  providerRun: RuntimeProviderRun | null
  waitingRoomState: WaitingRoomState
  defaultModel: string
  defaultEffort: string
}

export function deriveCurrentProviderSelection(options: ProviderSelectionOptions) {
  return {
    provider: options.providerRun?.provider ?? "opencode",
    model: options.providerRun?.model ?? options.waitingRoomState.modelId ?? options.defaultModel,
    effort: options.providerRun?.variant ?? options.waitingRoomState.effort ?? options.defaultEffort,
  }
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
  catalog: ProviderCatalog
}): PromptUsageMeta | null {
  const run = options.providerRun
  if (!run) {
    return null
  }

  return formatPromptUsageMeta(
    run.usage_tokens_total,
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
  if (options.daemonDisconnected) {
    return "disconnected"
  }
  if (options.working || options.hasActivePrompt || options.submitting || options.queueDepth > 0) {
    return "working"
  }
  return "idle"
}

export function deriveFooterHint(options: {
  fatalError: string | null
  activePromptId: string | null
  queueDepth: number
  statusLine: string
}): string {
  if (options.fatalError) {
    return options.fatalError
  }
  if (options.activePromptId) {
    return options.queueDepth > 0
      ? `Processing ${options.activePromptId}; ${options.queueDepth} queued.`
      : `Processing ${options.activePromptId}.`
  }
  return options.statusLine
}

export function deriveVisibleActivityLabel(options: {
  providerActivityLabel: string | null
  activeToolLabels: Iterable<string>
}) {
  const latestActiveToolLabel = Array.from(options.activeToolLabels).at(-1) ?? null
  return chooseVisibleActivityLabel(options.providerActivityLabel, latestActiveToolLabel)
}

export function deriveFocusedStatusBadge(options: {
  attached: boolean
  daemonDisconnected: boolean
  focusedAgent: SplitPaneFooterAgent | null
  focusedAgentActivityLabel: string | null
  streamingAgentId: string | null
}): FocusedStatusBadge {
  if (!options.attached) {
    return { label: "", tone: "idle" }
  }
  if (options.daemonDisconnected) {
    return { label: "DISCONNECTED", tone: "disconnected" }
  }
  return agentPaneStatusBadge(
    options.focusedAgent,
    options.focusedAgentActivityLabel,
    options.focusedAgent?.id === options.streamingAgentId,
  )
}

export function deriveAttachedFooterSummary(options: {
  session: RuntimeSession
  connectedClientCount: number
  multiAgentMode: boolean
  responseLayout: MultiAgentResponseLayout
  sessionStatusMode: SessionStatusMode
  hotkeyToggleLabel: string
}): string {
  const focusedAgent = options.session.agents.find(
    (agent) => agent.id === options.session.focused_agent_id,
  )
  const agentInfo = focusedAgent
    ? ` • Agent: ${formatAgentLabel(focusedAgent)}${focusedAgent.is_processing ? " [working]" : ""}`
    : ""
  const navigationInfo = options.multiAgentMode ? " • Tab cycles focus • Ctrl+P opens workflow" : ""

  return `Session ${options.session.alias ?? options.session.id} • ${options.connectedClientCount} ${options.connectedClientCount === 1 ? "CLI" : "CLIs"} connected • ${options.session.agents.length} ${options.session.agents.length === 1 ? "agent" : "agents"} in session${agentInfo}${options.sessionStatusMode === "working" ? " • Ctrl+C to stop" : ""}${navigationInfo} • ${options.hotkeyToggleLabel} hotkeys`
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

function formatAgentLabel(agent: RuntimeSession["agents"][number]) {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`
}

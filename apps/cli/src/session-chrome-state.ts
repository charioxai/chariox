import type { AgentInstance, RuntimeProviderRun, RuntimeSession, WorkspaceLiveSyncStatus } from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import {
  formatPromptMetaParts,
  formatPromptUsageMeta,
  type PromptMetaPart,
  type PromptUsageMeta,
} from "./prompt-meta.js"
import { chooseVisibleActivityLabel, getSessionStatusLabel } from "./runtime.js"
import type { StatusBadgeTone } from "./split-pane-footer.js"
import { agentPaneStatusBadge, type SplitPaneFooterAgent } from "./split-pane-footer.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type SessionStatusMode = "idle" | "working" | "disconnected"
export type StatusBadgePart = {
  label: string
  tone: StatusBadgeTone
}
export type FocusedStatusBadge = {
  label: string
  tone: StatusBadgeTone
  parts: StatusBadgePart[]
}

type ProviderSelectionOptions = {
  providerRun: RuntimeProviderRun | null
  focusedAgent?: AgentInstance | null
  waitingRoomState: WaitingRoomState
  defaultProvider?: string
  defaultModel: string
  defaultEffort: string
}

export function deriveCurrentProviderSelection(options: ProviderSelectionOptions) {
  return {
    provider: options.providerRun?.provider
      ?? normalizeProvider(options.focusedAgent?.provider)
      ?? options.waitingRoomState.providerId
      ?? options.defaultProvider
      ?? "opencode",
    model: options.providerRun?.model
      ?? options.focusedAgent?.model
      ?? options.waitingRoomState.modelId
      ?? options.defaultModel,
    effort: options.providerRun?.variant
      ?? options.focusedAgent?.effort
      ?? options.waitingRoomState.effort
      ?? options.defaultEffort,
  }
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
  catalog: ProviderCatalog
}): PromptUsageMeta | null {
  const run = options.providerRun
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

export type AgentBusyState = {
  id: string
  busy: boolean
}

export function deriveFocusedStatusBadge(options: {
  attached: boolean
  daemonDisconnected: boolean
  activeStatusLabel: string | null
  focusedBusy: boolean
  agents?: AgentBusyState[]
}): FocusedStatusBadge {
  if (!options.attached) {
    return statusBadge([])
  }
  if (options.daemonDisconnected) {
    return statusBadge([{ label: "DISCONNECTED", tone: "disconnected" }])
  }

  const agents = options.agents
  if (!agents || agents.length <= 1) {
    if (!options.focusedBusy) {
      return statusBadge([{ label: "IDLE", tone: "idle" }])
    }
    return statusBadge([{ label: getSessionStatusLabel("working", options.activeStatusLabel), tone: "working" }])
  }

  const idleCount = agents.filter((a) => !a.busy).length
  const workingCount = agents.length - idleCount

  if (workingCount === 0) {
    return statusBadge([{ label: `${agents.length} IDLE`, tone: "idle" }])
  }

  if (idleCount === 0) {
    return statusBadge([{ label: `${agents.length} WORKING`, tone: "working" }])
  }

  return statusBadge([
    { label: `${idleCount} IDLE`, tone: "idle" },
    { label: `${workingCount} WORKING`, tone: "working" },
  ])
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
  const navigationInfo = options.multiAgentMode ? " • Tab cycles focus • Ctrl+P opens workflow" : ""
  const agentInfo = formatVisibleAgentSummary(options.session)
  const workspaceLiveSyncInfo = options.workspaceLiveSyncStatus
    ? ` • sync ${options.workspaceLiveSyncStatus.footer_state}`
    : ""

  return `Session ${options.session.alias ?? options.session.id} • ${options.connectedClientCount} ${options.connectedClientCount === 1 ? "CLI" : "CLIs"} connected • ${agentInfo}${workspaceLiveSyncInfo}${options.sessionStatusMode === "working" ? " • Ctrl+C to stop" : ""}${navigationInfo} • ${options.hotkeyToggleLabel} hotkeys`
}

function formatVisibleAgentSummary(session: RuntimeSession): string {
  const counts = session.collaboration_agent_counts
  const visibleCount = counts?.owned_agent_count ?? session.agents.length
  const otherCount = counts?.other_user_agent_count ?? 0
  const collaboratorCount = counts?.collaborator_count ?? 0
  const ownLabel = `${visibleCount} visible ${visibleCount === 1 ? "agent" : "agents"}`
  const parts = [ownLabel]

  if (otherCount > 0) {
    parts.push(`${otherCount} collaborator ${otherCount === 1 ? "agent" : "agents"}`)
  }
  if (collaboratorCount > 0) {
    parts.push(`${collaboratorCount} ${collaboratorCount === 1 ? "collaborator" : "collaborators"}`)
  }

  return parts.join(" • ")
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

function statusBadge(parts: StatusBadgePart[]): FocusedStatusBadge {
  const label = parts.map((part) => part.label).join(" ")
  return {
    label,
    tone: parts.some((part) => part.tone === "working")
      ? "working"
      : parts[0]?.tone ?? "idle",
    parts,
  }
}

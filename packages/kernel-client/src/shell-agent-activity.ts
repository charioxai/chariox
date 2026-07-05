import type {
  AgentInstance,
  RuntimeInteraction,
  RuntimeProviderRun,
  RuntimeSession,
  WorkspaceLiveSyncStatus,
} from "./kernel-types.js"
import {
  agentRuntimeActivityResolvedStatus,
  agentRuntimeActivityIsBusy,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import type { AgentRuntimeActivityBusyInput, AgentRuntimeActivityProjection, AgentRuntimeActivityStatus } from "./agent-activity.js"
import {
  ACTIVE_STATUS_FALLBACK,
  normalizeProviderActivityLabel,
} from "./provider-status.js"
import { workspaceLiveSyncFooterSummary } from "./shell-workspace-format.js"
export {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
  type ActivePromptLifecycleRecord,
  type PromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
export {
  sessionActivePromptForAgent,
  sessionActivePromptIdForAgent,
  sessionHasActivePrompt,
  sessionPromptForAgent,
  sessionPromptStateForAgent,
} from "./session-prompt-identity.js"
export {
  sessionAgentIsBusy,
  sessionHasAgentRuntimeProjection,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
  sessionPromptWorkByAgent,
  sessionPromptWorkSummary,
  type SessionPromptWorkSummary,
} from "./session-prompt-work.js"
export {
  deriveAllAgentsBusyState,
  deriveFocusedActivityLabel,
  deriveFocusedAgentBusy,
  nextAgentActivityLabels,
  nextAgentBusyLatches,
  readAgentBusyLatch,
  resolveActiveToolLabelForAgent,
  resolveSessionStreamingAgentId,
  sessionFocusedAgentId,
  sessionRuntimeTransitionState,
  sessionShouldConfirmIdleTurnCompletion,
  sessionWorkingStateAfterPromptWork,
  shouldPreserveAgentActivityLabel,
  turnCompletionDelayMs,
  type AgentBusyState,
  type AgentToolActivityUpdate,
  type SessionIdleTurnCompletionInput,
  type SessionRuntimeTransitionOptions,
  type SessionRuntimeTransitionState,
  type SessionStreamingAgent,
  type TurnCompletionDelayInput,
} from "./session-runtime-transition.js"

export type AgentPromptStateLike = {
  readonly active_prompt?: unknown | null
  readonly queued_prompts?: readonly unknown[] | null
}

export type AgentRuntimeProjectionContext = {
  readonly agentActivity?: Record<string, AgentRuntimeActivityBusyInput> | null | undefined
  readonly promptStates?: Record<string, AgentPromptStateLike | null> | null | undefined
}

export type AgentRuntimeDisplayState = AgentInstance["state"] | "Done"

export type SessionStatusBadgeTone = "idle" | "working" | "disconnected" | "error"
export type SessionStatusMode = "idle" | "working" | "disconnected"

export type SessionStatusBadgePart = {
  label: string
  tone: SessionStatusBadgeTone
}

export type SessionFocusedStatusBadge = {
  label: string
  tone: SessionStatusBadgeTone
  parts: SessionStatusBadgePart[]
}

export type SessionAgentPaneStatusBadge = {
  label: string
  tone: SessionStatusBadgeTone
}

export type SessionAgentPaneStatusInput = {
  readonly state?: string | null
  readonly is_processing?: boolean | null
}

export type SessionAgentBusyState = {
  id: string
  busy: boolean
}

export function sessionStatusMode(options: {
  readonly daemonDisconnected: boolean
  readonly working: boolean
  readonly hasActivePrompt: boolean
  readonly submitting: boolean
  readonly queueDepth: number
}): SessionStatusMode {
  if (options.daemonDisconnected) {
    return "disconnected"
  }
  if (options.working || options.hasActivePrompt || options.submitting || options.queueDepth > 0) {
    return "working"
  }
  return "idle"
}

export function sessionFooterHint(options: {
  readonly fatalError: string | null
  readonly activePromptId: string | null
  readonly queueDepth: number
  readonly statusLine: string
}): string {
  if (options.fatalError) {
    return options.fatalError
  }
  if (options.activePromptId) {
    return options.queueDepth > 0
      ? `Processing ${options.activePromptId}; ${options.queueDepth} queued.`
      : `Processing ${options.activePromptId}.`
  }
  if (options.queueDepth > 0) {
    return `${options.queueDepth} queued prompt${options.queueDepth === 1 ? "" : "s"}.`
  }
  return options.statusLine
}

export function sessionAttachedFooterSummary(options: {
  readonly session: RuntimeSession
  readonly connectedClientCount: number
  readonly multiAgentMode: boolean
  readonly sessionStatusMode: SessionStatusMode
  readonly hotkeyToggleLabel: string
  readonly workspaceLiveSyncStatus?: WorkspaceLiveSyncStatus | null
}): string {
  const navigationInfo = options.multiAgentMode ? " • Tab cycles focus • Ctrl+P opens workflow" : ""
  const agentInfo = sessionVisibleAgentSummary(options.session)
  const workspaceLiveSyncInfo = options.workspaceLiveSyncStatus
    ? ` • ${workspaceLiveSyncFooterSummary(options.workspaceLiveSyncStatus)}`
    : ""

  return `Session ${options.session.alias ?? options.session.id} • ${options.connectedClientCount} ${options.connectedClientCount === 1 ? "CLI" : "CLIs"} connected • ${agentInfo}${workspaceLiveSyncInfo}${options.sessionStatusMode === "working" ? " • Ctrl+C to stop" : ""}${navigationInfo} • ${options.hotkeyToggleLabel} hotkeys`
}

export function sessionVisibleAgentSummary(session: RuntimeSession): string {
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

export function sessionAgentRuntimeActivityProjection(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): AgentRuntimeActivityProjection {
  const activity = agentId ? session?.agent_activity?.[agentId] : null
  return projectAgentRuntimeActivity(activity)
}

export function sessionAgentRuntimeActivityStatus(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): AgentRuntimeActivityStatus {
  const activity = agentId ? session?.agent_activity?.[agentId] : null
  return agentRuntimeActivityResolvedStatus(activity)
}

export function sessionFocusedStatusBadge(options: {
  readonly attached: boolean
  readonly daemonDisconnected: boolean
  readonly activeStatusLabel: string | null
  readonly focusedBusy: boolean
  readonly agents?: readonly SessionAgentBusyState[]
}): SessionFocusedStatusBadge {
  if (!options.attached) {
    return sessionStatusBadge([])
  }
  if (options.daemonDisconnected) {
    return sessionStatusBadge([{ label: "DISCONNECTED", tone: "disconnected" }])
  }

  const agents = options.agents
  if (!agents || agents.length <= 1) {
    if (!options.focusedBusy) {
      return sessionStatusBadge([{ label: "IDLE", tone: "idle" }])
    }
    return sessionStatusBadge([{
      label: formatSessionWorkingStatusLabel(options.activeStatusLabel),
      tone: "working",
    }])
  }

  const idleCount = agents.filter((agent) => !agent.busy).length
  const workingCount = agents.length - idleCount

  if (workingCount === 0) {
    return sessionStatusBadge([{ label: `${agents.length} IDLE`, tone: "idle" }])
  }

  if (idleCount === 0) {
    return sessionStatusBadge([{ label: `${agents.length} WORKING`, tone: "working" }])
  }

  return sessionStatusBadge([
    { label: `${idleCount} IDLE`, tone: "idle" },
    { label: `${workingCount} WORKING`, tone: "working" },
  ])
}

export function sessionStatusLabel(
  mode: SessionStatusMode,
  activity: string | null,
): string {
  if (mode === "disconnected") {
    return "DISCONNECTED"
  }
  if (mode === "idle") {
    return "IDLE"
  }
  return formatSessionWorkingStatusLabel(activity)
}

export function sessionAgentPaneStatusBadge(options: {
  readonly agent: SessionAgentPaneStatusInput | null
  readonly activeLabel: string | null
  readonly hasPromptWork?: boolean
  readonly isStreaming?: boolean
  readonly busyLatch?: boolean
  readonly useLegacyAgentProcessingState?: boolean
}): SessionAgentPaneStatusBadge {
  const agent = options.agent
  if (!agent) {
    return { label: "", tone: "idle" }
  }
  if (agent.state === "Error") {
    return { label: "ERROR", tone: "error" }
  }
  if (options.activeLabel) {
    return { label: sessionStatusLabel("working", options.activeLabel), tone: "working" }
  }
  const useLegacyAgentProcessingState = options.useLegacyAgentProcessingState ?? true
  const legacyAgentBusy = useLegacyAgentProcessingState
    && (agent.is_processing === true || agent.state === "Working")
  if (
    options.hasPromptWork === true
    || legacyAgentBusy
    || options.isStreaming === true
    || options.busyLatch === true
  ) {
    return { label: sessionStatusLabel("working", null), tone: "working" }
  }
  return { label: "IDLE", tone: "idle" }
}

function sessionStatusBadge(parts: SessionStatusBadgePart[]): SessionFocusedStatusBadge {
  return {
    label: parts.map((part) => part.label).join(" "),
    tone: parts.some((part) => part.tone === "working")
      ? "working"
      : parts[0]?.tone ?? "idle",
    parts,
  }
}

function formatSessionWorkingStatusLabel(activity: string | null): string {
  return (normalizeProviderActivityLabel(activity) ?? ACTIVE_STATUS_FALLBACK).trim().toUpperCase()
}

export function sessionActiveInteractionForAgent(
  session: Pick<RuntimeSession, "active_interactions">,
  agentId: string | null | undefined,
): RuntimeInteraction | null {
  if (!agentId) {
    return null
  }
  return session.active_interactions?.find((interaction) => interaction.agent_id === agentId) ?? null
}

export function runtimeProviderRunForAgent(
  run: RuntimeProviderRun | null,
  agentId: string | null | undefined,
): RuntimeProviderRun | null {
  return run && run.agent_instance_id === agentId ? run : null
}

export function sessionAgentRuntimeState(
  session: RuntimeSession | null | undefined,
  agent: AgentInstance,
): AgentInstance["state"] {
  if (!session) {
    return legacyAgentRuntimeState(agent)
  }
  return agentRuntimeStateFromProjection(agent, {
    agentActivity: session.agent_activity,
    promptStates: session.prompt_states,
  })
}

export function sessionAgentRuntimeDisplayState(
  session: RuntimeSession | null | undefined,
  agent: AgentInstance,
): AgentRuntimeDisplayState {
  const runtimeState = sessionAgentRuntimeState(session, agent)
  return sessionAgentHasUnreadIdleOutput(session, agent.id) && runtimeState === "Idle" ? "Done" : runtimeState
}

export function agentRuntimeStateFromProjection(
  agent: AgentInstance,
  context: AgentRuntimeProjectionContext,
): AgentInstance["state"] {
  if (context.agentActivity && !(agent.id in context.agentActivity)) {
    return agent.state === "Error" ? "Error" : "Idle"
  }
  const projected = context.agentActivity?.[agent.id]
  if (projected) {
    const resolvedStatus = agentRuntimeActivityResolvedStatus(projected)
    if (resolvedStatus === "error") {
      return "Error"
    }
    return resolvedStatus === "working" ? "Working" : "Idle"
  }
  const promptState = context.promptStates?.[agent.id]
  if (promptState !== undefined) {
    if (agent.state === "Error") {
      return "Error"
    }
    return Boolean(promptState?.active_prompt) || Boolean(promptState?.queued_prompts?.length)
      ? "Working"
      : "Idle"
  }
  return legacyAgentRuntimeState(agent)
}

export function sessionAgentHasUnreadIdleOutput(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): boolean {
  if (!session || !agentId || session.focused_agent_id === agentId) {
    return false
  }
  const activity = session.agent_activity?.[agentId]
  return projectAgentRuntimeActivity(activity).unreadIdleOutput
}

function legacyAgentRuntimeState(agent: AgentInstance): AgentInstance["state"] {
  return agent.is_processing && agent.state !== "Error" ? "Working" : agent.state
}

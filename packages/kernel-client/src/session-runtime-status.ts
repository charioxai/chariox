import type {
  AgentInstance,
  RuntimeSession,
} from "./kernel-types.js"
import {
  agentLegacyProcessingStateIsBusy,
  agentRuntimeActivityProjectionResolvedStatus,
  agentRuntimeActivityResolvedStatus,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import type { AgentRuntimeActivityBusyInput, AgentRuntimeActivityProjection, AgentRuntimeActivityStatus } from "./agent-activity.js"
import {
  ACTIVE_STATUS_FALLBACK,
  normalizeProviderActivityLabel,
} from "./provider-status.js"
import {
  sessionProjectedPromptActivityForAgent,
  sessionPromptStateRecordForAgent,
  type SessionAgentPromptStateLike,
} from "./session-agent-prompt-state.js"

export type AgentRuntimeProjectionContext = {
  readonly agentActivity?: Record<string, AgentRuntimeActivityBusyInput> | null | undefined
  readonly promptStates?: Record<string, SessionAgentPromptStateLike | null> | null | undefined
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

export type SessionAgentRuntimeDisplayState = {
  id: string
  state: AgentRuntimeDisplayState
}

export function sessionStatusMode(options: {
  readonly daemonDisconnected: boolean
  readonly working: boolean
  readonly hasActiveTurnWork: boolean
  readonly submitting: boolean
  readonly queueDepth: number
}): SessionStatusMode {
  if (options.daemonDisconnected) {
    return "disconnected"
  }
  if (options.working || options.hasActiveTurnWork || options.submitting) {
    return "working"
  }
  return "idle"
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
    && agentLegacyProcessingStateIsBusy(agent)
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

export function sessionAgentPaneStatusBadgeForSession(options: {
  readonly session: RuntimeSession | null | undefined
  readonly agent: AgentInstance | null | undefined
  readonly activeLabel: string | null
  readonly isStreaming?: boolean
  readonly busyLatch?: boolean
}): SessionAgentPaneStatusBadge {
  const agent = options.agent
  if (!agent) {
    return sessionAgentPaneStatusBadge({ agent: null, activeLabel: options.activeLabel })
  }
  const runtimeState = sessionAgentRuntimeState(options.session, agent)
  return sessionAgentPaneStatusBadge({
    agent: { state: runtimeState, is_processing: false },
    activeLabel: options.activeLabel,
    hasPromptWork: runtimeState === "Working",
    isStreaming: options.isStreaming ?? false,
    busyLatch: options.busyLatch ?? false,
    useLegacyAgentProcessingState: false,
  })
}

export function sessionAgentRuntimeActivityProjection(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): AgentRuntimeActivityProjection {
  if (!session || !agentId) {
    return projectAgentRuntimeActivity(null)
  }
  const projection = sessionProjectedPromptActivityForAgent(session, agentId)
  if (!projection || projection === "idle" || projection === "not_found") {
    return projectAgentRuntimeActivity(null)
  }
  return projection
}

export function sessionAgentRuntimeActivityStatus(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): AgentRuntimeActivityStatus {
  return agentRuntimeActivityProjectionResolvedStatus(
    sessionAgentRuntimeActivityProjection(session, agentId),
  )
}

export function sessionAgentRuntimeState(
  session: RuntimeSession | null | undefined,
  agent: AgentInstance,
): AgentInstance["state"] {
  if (!session) {
    return legacyAgentRuntimeState(agent)
  }
  const projection = sessionProjectedPromptActivityForAgent(session, agent.id)
  if (projection && projection !== "idle" && projection !== "not_found") {
    const resolvedStatus = agentRuntimeActivityProjectionResolvedStatus(projection)
    if (resolvedStatus === "error") {
      return "Error"
    }
    return resolvedStatus === "working" ? "Working" : "Idle"
  }
  if (projection === "idle" || projection === "not_found") {
    return agent.state === "Error" ? "Error" : "Idle"
  }
  const promptState = sessionPromptStateRecordForAgent(session, agent.id)
  if (promptState !== undefined) {
    if (agent.state === "Error") {
      return "Error"
    }
    return promptStateHasActivePrompt(promptState)
      ? "Working"
      : "Idle"
  }
  return legacyAgentRuntimeState(agent)
}

export function sessionAgentRuntimeDisplayState(
  session: RuntimeSession | null | undefined,
  agent: AgentInstance,
): AgentRuntimeDisplayState {
  const runtimeState = sessionAgentRuntimeState(session, agent)
  return sessionAgentHasUnreadIdleOutput(session, agent.id) && runtimeState === "Idle" ? "Done" : runtimeState
}

export function sessionAgentRuntimeDisplayStates(
  session: RuntimeSession | null | undefined,
): readonly SessionAgentRuntimeDisplayState[] {
  if (!session) {
    return []
  }
  return session.agents.map((agent) => ({
    id: agent.id,
    state: sessionAgentRuntimeDisplayState(session, agent),
  }))
}

export function sessionAgentRuntimeDisplayStateByAgent(
  session: RuntimeSession | null | undefined,
): Readonly<Record<string, AgentRuntimeDisplayState>> {
  return Object.fromEntries(
    sessionAgentRuntimeDisplayStates(session).map((entry) => [entry.id, entry.state]),
  )
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
    return promptStateHasActivePrompt(promptState)
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
  const projection = sessionProjectedPromptActivityForAgent(session, agentId)
  if (!projection || projection === "idle" || projection === "not_found") {
    return false
  }
  return projection.unreadIdleOutput
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

function promptStateHasActivePrompt(state: SessionAgentPromptStateLike | null | undefined): boolean {
  return Boolean(state?.active_prompt)
}

function legacyAgentRuntimeState(agent: AgentInstance): AgentInstance["state"] {
  return agent.is_processing && agent.state !== "Error" ? "Working" : agent.state
}

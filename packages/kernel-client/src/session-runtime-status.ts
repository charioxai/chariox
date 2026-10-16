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
import { agentTaskStatus, agentWaitCountdown } from "@chariox/tool-display/agent-task-status"

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
  deadline_ms?: number
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
  readonly nowMs?: number
  readonly taskStatus?: SessionAgentPaneStatusBadge | null
}): SessionFocusedStatusBadge {
  if (!options.attached) {
    return sessionStatusBadge([])
  }
  if (options.daemonDisconnected) {
    return sessionStatusBadge([{ label: "DISCONNECTED", tone: "disconnected" }])
  }

  const agents = options.agents
  if (options.taskStatus) {
    const task = options.taskStatus
    // MP-08/MP-09/MP-10/MP-11 G13: reserve footer space for the actual deadline.
    const label = task.deadline_ms !== undefined
      ? `WAITING ${agentWaitCountdown(task.deadline_ms, options.nowMs ?? Date.now())} UNTIL ${new Date(task.deadline_ms).toISOString().slice(11, 19)}Z`
      : task.label.length > 24 ? `${task.label.slice(0, 23)}…` : task.label
    return sessionStatusBadge([{ label, tone: task.tone }])
  }
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
  const taskStatus = sessionAgentTaskStatus(options.session, agent.id)
  if (taskStatus) return taskStatus
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
    return agent.state === "Error" ? "Error" : "Idle"
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
  return agent.state === "Error" ? "Error" : "Idle"
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
  if (context.promptStates) {
    return agent.state === "Error" ? "Error" : "Idle"
  }
  return agent.state === "Error" ? "Error" : "Idle"
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

// MP-08 / MP-09 / MP-10 / MP-11 A02: older unfinished tasks survive newer answers.
export function sessionAgentTaskStatus(session: RuntimeSession | null | undefined, agentId: string, nowMs = Date.now()): SessionAgentPaneStatusBadge | null {
  const tasks = (session?.agent_tasks ?? []).filter((task) => task.agent_id === agentId)
  if (!tasks.length) return null
  const unfinished = tasks.filter((task) => task.state !== "done" && task.state !== "cancelled")
  const goals = tasks.filter((task) => task.goal)
  const task = goals.find((task) => task.state !== "done" && task.state !== "cancelled")
    ?? unfinished.find((task) => task.state === "blocked")
    ?? unfinished.find((task) => task.state === "working")
    ?? unfinished[0] ?? goals[goals.length - 1] ?? tasks[tasks.length - 1]!
  const open = (task.goal ? task.obligations : tasks.flatMap((task) => task.obligations)).filter((item) => ["open", "settling", "failed"].includes(item.status)).length
  const suffix = open ? ` (${open} obligations)` : ""
  if (task.goal) {
    const goal = task.goal
    const phase = goal.paused ? "PAUSED" : task.state === "working" && goal.evaluation?.result === null && goal.evaluation.prompt_id === (task.pending_prompt_id ?? task.prompt_id) ? "EVALUATING" : task.state.toUpperCase()
    const elapsed = Number.isFinite(goal.state_since_ms) ? Math.max(0, nowMs - goal.state_since_ms) : 0
    const active = goal.active_elapsed_ms + (task.state === "working" ? elapsed : 0)
    const waiting = goal.waiting_elapsed_ms + (task.state === "waiting" ? elapsed : 0)
    const seconds = (ms: number) => `${Math.floor(ms / 1000)}s`
    const deadline = goal.next_check_in_ms ? `${Math.max(0, Math.ceil((goal.next_check_in_ms - nowMs) / 1000))}s (${new Date(goal.next_check_in_ms).toISOString()})` : "none"
    const sources = task.wait?.registration_ids.length ?? 0
    const objective = [...goal.objective.replace(/\s+/g, " ").trim()]
    const title = objective.length > 160 ? objective.slice(0, 159).join("") + "…" : objective.join("")
    return { label: `GOAL ${phase}: ${title}\nnext check ${deadline} · active ${seconds(active)} · wait ${seconds(waiting)} · ${goal.check_in_minutes}m · check ${goal.check_count}\nsources ${sources} · progress ${task.progress_sequence ?? 0} at ${task.last_progress_at_ms ? new Date(task.last_progress_at_ms).toISOString() : "none"} · ${open} obligations${task.reason ? ` · ${task.reason}` : ""}\n${goal.id} revision ${task.revision} · /goal pause | resume | cancel | interval <minutes> | edit <objective>`, tone: task.state === "blocked" ? "error" : task.state === "working" ? "working" : "idle" }
  }
  const status = agentTaskStatus(session, agentId)
  return status ? { label: status.label, tone: status.tone, ...(status.deadline_ms !== undefined ? { deadline_ms: status.deadline_ms } : {}) } : null
}

// MP-08 / MP-09 / MP-10 / MP-11 A12: client display clock, never scheduling authority.
export function sessionHasActiveGoalClock(session: RuntimeSession | null | undefined): boolean {
  return (session?.agent_tasks ?? []).some(task => task.goal && !task.goal.paused && (task.state === "working" || task.state === "waiting"))
}

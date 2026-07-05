import type {
  AgentInstance,
  AgentPromptState,
  PromptQueueItem,
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
import {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
import {
  sessionAgentIsBusy,
  sessionHasAgentRuntimeProjection,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
  sessionPromptWorkByAgent,
  sessionPromptWorkSummary,
} from "./session-prompt-work.js"
export {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
  type ActivePromptLifecycleRecord,
  type PromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
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

export type SessionIdleTurnCompletionInput = {
  readonly nextSession: RuntimeSession
  readonly currentWorking: boolean
  readonly currentSubmitting: boolean
  readonly currentBusyLatches: Record<string, boolean>
  readonly currentStreamingAgentId: string | null
  readonly currentProviderActivityLabel: string | null
  readonly currentActiveStatusLabel: string | null
}

export type SessionRuntimeTransitionOptions = {
  readonly currentSession: RuntimeSession
  readonly nextSession: RuntimeSession
  readonly currentWorking: boolean
  readonly currentStreamingAgentId: string | null
  readonly currentAgentActivityLabels: Record<string, string | null>
}

export type SessionRuntimeTransitionState = {
  readonly nextFocusedAgentId: string | null
  readonly nextHasPromptWork: boolean
  readonly nextStreamingAgentId: string | null
  readonly nextFocusedActivityLabel: string | null
  readonly nextAgentActivityLabels: Record<string, string | null>
  readonly nextWorking: boolean
  readonly previousAgentSignature: string
  readonly nextAgentSignature: string
}

export type AgentBusyState = {
  readonly id: string
  readonly busy: boolean
}

export type AgentToolActivityUpdate = {
  readonly tool?: string | null
  readonly status?: string | null
}

export type AgentPromptStateLike = {
  readonly active_prompt?: unknown | null
  readonly queued_prompts?: readonly unknown[] | null
}

export type AgentRuntimeProjectionContext = {
  readonly agentActivity?: Record<string, AgentRuntimeActivityBusyInput> | null | undefined
  readonly promptStates?: Record<string, AgentPromptStateLike | null> | null | undefined
}

export type AgentRuntimeDisplayState = AgentInstance["state"] | "Done"

export type SessionStreamingAgent = Pick<AgentInstance, "id" | "is_processing" | "state">

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

export function sessionFocusedAgentId(
  session: Pick<RuntimeSession, "agents" | "focused_agent_id">,
): string | null {
  const focusedAgentId = session.focused_agent_id?.trim()
  if (focusedAgentId && session.agents.some((agent) => agent.id === focusedAgentId)) {
    return focusedAgentId
  }
  if (focusedAgentId) {
    return null
  }
  return session.agents[0]?.id ?? null
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

export function resolveSessionStreamingAgentId(
  agents: ReadonlyArray<SessionStreamingAgent>,
  activePromptTargetAgentId: string | null,
  sessionHasPromptWork: boolean,
  currentWorking: boolean,
  previousStreamingAgentId: string | null,
  useLegacyProcessingState = true,
): string | null {
  const processingAgentId = useLegacyProcessingState
    ? agents.find((agent) => agent.is_processing || agent.state === "Working")?.id ?? null
    : null
  if (processingAgentId) {
    return processingAgentId
  }
  if (activePromptTargetAgentId && agents.some((agent) => agent.id === activePromptTargetAgentId)) {
    return activePromptTargetAgentId
  }
  if (
    (sessionHasPromptWork || (currentWorking && useLegacyProcessingState))
    && previousStreamingAgentId
    && agents.some((agent) => agent.id === previousStreamingAgentId)
  ) {
    return previousStreamingAgentId
  }
  return null
}

export function sessionRuntimeTransitionState(
  options: SessionRuntimeTransitionOptions,
): SessionRuntimeTransitionState {
  const previousAgentSignature = options.currentSession.agents
    .map((agent) => agent.id)
    .join(",")
  const nextAgentSignature = options.nextSession.agents.map((agent) => agent.id).join(",")
  const nextFocusedAgentId = sessionFocusedAgentId(options.nextSession)
  const nextHasPromptWork = sessionHasPromptWork(options.nextSession)
  const projectedStreamingAgentId = sessionProjectedStreamingAgentId(options.nextSession)
  const nextStreamingAgentId = options.nextSession.agent_activity
    ? projectedStreamingAgentId
    : resolveSessionStreamingAgentId(
      options.nextSession.agents,
      projectedStreamingAgentId,
      nextHasPromptWork,
      options.currentWorking,
      options.currentStreamingAgentId,
      !options.nextSession.prompt_states,
    )
  const nextAgentActivityLabels: Record<string, string | null> = {}
  for (const agent of options.nextSession.agents) {
    const legacyAgentBusy = !sessionHasAgentRuntimeProjection(options.nextSession)
      && (agent.is_processing || agent.state === "Working")
    nextAgentActivityLabels[agent.id] =
      legacyAgentBusy
        || agent.id === nextStreamingAgentId
        || sessionAgentIsBusy(options.nextSession, agent.id)
        ? (options.currentAgentActivityLabels[agent.id] ?? null)
        : null
  }
  const nextFocusedActivityLabel = nextFocusedAgentId
    ? nextAgentActivityLabels[nextFocusedAgentId] ?? null
    : null

  return {
    nextFocusedAgentId,
    nextHasPromptWork,
    nextStreamingAgentId,
    nextFocusedActivityLabel,
    nextAgentActivityLabels,
    nextWorking: sessionWorkingStateAfterPromptWork(options.currentWorking, nextHasPromptWork),
    previousAgentSignature,
    nextAgentSignature,
  }
}

export function sessionWorkingStateAfterPromptWork(
  currentWorking: boolean,
  sessionHasPromptWork: boolean,
): boolean {
  return sessionHasPromptWork ? true : currentWorking
}

export function readAgentBusyLatch(
  latches: Record<string, boolean>,
  agentId: string | null | undefined,
): boolean {
  return agentId ? (latches[agentId] ?? false) : false
}

export function nextAgentBusyLatches(
  current: Record<string, boolean>,
  agentId: string | null | undefined,
  busy: boolean,
): Record<string, boolean> {
  if (!agentId || (current[agentId] ?? false) === busy) {
    return current
  }
  if (busy) {
    return {
      ...current,
      [agentId]: true,
    }
  }
  const next = { ...current }
  delete next[agentId]
  return next
}

export function shouldPreserveAgentActivityLabel(options: {
  readonly agentId: string | null | undefined
  readonly session: RuntimeSession
  readonly streamingAgentId: string | null
}): boolean {
  const agentId = options.agentId
  if (!agentId) {
    return false
  }
  return options.streamingAgentId === agentId
    || sessionAgentIsBusy(options.session, agentId)
    || (!sessionHasAgentRuntimeProjection(options.session)
      && options.session.agents.some((agent) => agent.id === agentId && (agent.is_processing || agent.state === "Working")))
}

export function nextAgentActivityLabels(
  current: Record<string, string | null>,
  agentId: string | null | undefined,
  nextLabel: string | null,
  preserveCurrent: boolean,
): Record<string, string | null> {
  if (!agentId) {
    return current
  }
  return {
    ...current,
    [agentId]: nextLabel ?? (preserveCurrent ? (current[agentId] ?? null) : null),
  }
}

export function deriveFocusedActivityLabel(options: {
  readonly focusedAgentId: string | null
  readonly activeToolLabel: string | null
  readonly agentActivityLabel: string | null
}): string | null {
  return options.focusedAgentId ? (options.activeToolLabel ?? options.agentActivityLabel) : null
}

export function resolveActiveToolLabelForAgent(options: {
  readonly agentId: string | null | undefined
  readonly visibleTranscriptAgentId: string | null
  readonly activeToolLabels: Iterable<string>
  readonly agentPaneToolUpdates: Iterable<AgentToolActivityUpdate> | null | undefined
  readonly toolActivityLabel: (tool?: string | null) => string | null
}): string | null {
  const agentId = options.agentId
  if (!agentId) {
    return null
  }
  if (agentId === options.visibleTranscriptAgentId) {
    return Array.from(options.activeToolLabels).at(-1) ?? null
  }
  const labels = Array.from(options.agentPaneToolUpdates ?? [])
    .filter((update) => update.status !== "completed" && update.status !== "error" && update.status !== "cancelled")
    .map((update) => options.toolActivityLabel(update.tool))
    .filter((label): label is string => Boolean(label))
  return labels.at(-1) ?? null
}

export function deriveFocusedAgentBusy(options: {
  readonly focusedAgentId: string | null
  readonly submitting: boolean
  readonly submittingAgentId: string | null
  readonly session: RuntimeSession
  readonly streamingAgentId: string | null
  readonly focusedActivityLabel: string | null
  readonly agentBusyLatches: Record<string, boolean>
}): boolean {
  const agentId = options.focusedAgentId
  if (!agentId) {
    return false
  }
  const focused = options.session.agents.find((agent) => agent.id === agentId) ?? null
  const allowLegacyProcessing = !sessionHasAgentRuntimeProjection(options.session)
  return (options.submitting && options.submittingAgentId === agentId)
    || sessionAgentIsBusy(options.session, agentId)
    || options.streamingAgentId === agentId
    || Boolean(options.focusedActivityLabel)
    || readAgentBusyLatch(options.agentBusyLatches, agentId)
    || Boolean(allowLegacyProcessing && focused && (focused.is_processing || focused.state === "Working"))
}

export function deriveAllAgentsBusyState(options: {
  readonly submitting: boolean
  readonly submittingAgentId: string | null
  readonly session: RuntimeSession
  readonly streamingAgentId: string | null
  readonly agentActivityLabels: Record<string, string | null>
  readonly agentBusyLatches: Record<string, boolean>
}): AgentBusyState[] {
  return options.session.agents.map((agent) => {
    const agentId = agent.id
    const allowLegacyProcessing = !sessionHasAgentRuntimeProjection(options.session)
    const isBusy = (options.submitting && options.submittingAgentId === agentId)
      || sessionAgentIsBusy(options.session, agentId)
      || options.streamingAgentId === agentId
      || Boolean(options.agentActivityLabels[agentId])
      || readAgentBusyLatch(options.agentBusyLatches, agentId)
      || (allowLegacyProcessing && (agent.is_processing || agent.state === "Working"))
    return { id: agentId, busy: isBusy }
  })
}

export function sessionShouldConfirmIdleTurnCompletion(options: SessionIdleTurnCompletionInput): boolean {
  if (sessionHasPromptWork(options.nextSession) || sessionHasProcessingAgent(options.nextSession)) {
    return false
  }
  return options.currentWorking
    || options.currentSubmitting
    || Object.values(options.currentBusyLatches).some(Boolean)
    || options.currentStreamingAgentId !== null
    || options.currentProviderActivityLabel !== null
    || options.currentActiveStatusLabel !== null
}

export type TurnCompletionDelayInput = {
  readonly sessionHasPromptWork: boolean
  readonly pendingTerminalRecordCount: number
  readonly pendingTerminalRecordFlush: boolean
  readonly lastTurnActivityAt: number
  readonly now: number
  readonly quietWindowMs: number
}

export function turnCompletionDelayMs(options: TurnCompletionDelayInput): number | null {
  if (
    options.sessionHasPromptWork
    || options.pendingTerminalRecordCount > 0
    || options.pendingTerminalRecordFlush
  ) {
    return null
  }
  return Math.max(0, options.quietWindowMs - Math.max(0, options.now - options.lastTurnActivityAt))
}

export function sessionPromptStateForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): AgentPromptState | null {
  if (!agentId) {
    return null
  }
  const promptStates = session.prompt_states
  if (promptStates) {
    return Object.prototype.hasOwnProperty.call(promptStates, agentId)
      ? normalizeAgentPromptState(promptStates[agentId])
      : null
  }
  if (session.agent_activity) {
    return null
  }
  const activePrompt = session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
  const queuedPrompts = session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId)
  if (activePrompt || queuedPrompts.length > 0) {
    return {
      active_prompt: activePrompt,
      queued_prompts: queuedPrompts,
    }
  }
  return null
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

export function sessionHasActivePrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  if (!sessionHasAgent(session, agentId)) {
    return false
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const projection = projectAgentRuntimeActivity(projected)
    if (projection.activeTurnPromptId) {
      return projection.activeTurnPromptId === promptId
    }
    if (!projection.busy) {
      return false
    }
  }
  return legacySessionHasPrompt(session, agentId, promptId)
}

export function sessionPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  if (!sessionHasAgent(session, agentId)) {
    return null
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return null
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const projection = projectAgentRuntimeActivity(projected)
    const activeTurnPromptId = projection.activeTurnPromptId ?? null
    if (activeTurnPromptId) {
      const prompt = legacyPromptForAgent(session, agentId)
      return prompt?.id === activeTurnPromptId ? prompt : null
    }
    if (!projection.busy) {
      return null
    }
  }
  return legacyPromptForAgent(session, agentId)
}

export function sessionActivePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  if (!sessionHasAgent(session, agentId)) {
    return null
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return null
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const projection = projectAgentRuntimeActivity(projected)
    const activeTurnPromptId = projection.activeTurnPromptId ?? null
    if (activeTurnPromptId) {
      const prompt = activePromptForAgent(session, agentId)
      return prompt?.id === activeTurnPromptId ? prompt : null
    }
    if (!projection.busy) {
      return null
    }
  }
  return activePromptForAgent(session, agentId)
}

export function sessionActivePromptIdForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): string | null {
  if (agentId) {
    const projected = session.agent_activity?.[agentId]
    const projection = projectAgentRuntimeActivity(projected)
    const projectedPromptId = projection.activeTurnPromptId ?? null
    if (projectedPromptId) {
      return projectedPromptId
    }
    if (session.agent_activity && !projection.busy) {
      return null
    }
    return activePromptForAgent(session, agentId)?.id ?? null
  }

  const records = sessionActivePromptLifecycleRecords(session)
  return records.length === 1 ? records[0]?.id ?? null : null
}

function legacySessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt?.id === promptId
      || Boolean(promptState?.queued_prompts?.some((prompt) => prompt.id === promptId))
  }
  return Boolean(session.active_prompt?.target_agent_id === agentId && session.active_prompt.id === promptId)
    || session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId && prompt.id === promptId)
}

function legacyPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt
      ?? promptState?.queued_prompts?.[promptState.queued_prompts.length - 1]
      ?? null
  }
  return (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    ?? [...session.queued_prompts].reverse().find((prompt) => prompt.target_agent_id === agentId)
    ?? null
}

function activePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  if (session.agent_activity) {
    return null
  }
  return session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
}

function promptStateForAgent(session: RuntimeSession, agentId: string) {
  const promptStates = session.prompt_states
  if (!promptStates) {
    return undefined
  }
  if (!Object.prototype.hasOwnProperty.call(promptStates, agentId)) {
    return null
  }
  return promptStates[agentId] ?? null
}

function normalizeAgentPromptState(state: Partial<AgentPromptState> | null | undefined): AgentPromptState {
  return {
    active_prompt: state?.active_prompt ?? null,
    queued_prompts: Array.isArray(state?.queued_prompts) ? state.queued_prompts : [],
  }
}

function sessionHasAgent(session: RuntimeSession, agentId: string): boolean {
  return sessionAgentIds(session).has(agentId)
}

function sessionAgentIds(session: RuntimeSession): ReadonlySet<string> {
  return new Set(session.agents.map((agent) => agent.id))
}

function legacyAgentRuntimeState(agent: AgentInstance): AgentInstance["state"] {
  return agent.is_processing && agent.state !== "Error" ? "Working" : agent.state
}

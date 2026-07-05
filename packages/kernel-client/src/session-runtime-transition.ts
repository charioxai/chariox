import type {
  AgentInstance,
  RuntimeSession,
} from "./kernel-types.js"
import {
  sessionAgentIsBusy,
  sessionHasAgentRuntimeProjection,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
} from "./session-prompt-work.js"
import {
  getToolActivityLabel,
} from "./provider-status.js"

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

export type SessionStreamingAgent = Pick<AgentInstance, "id" | "is_processing" | "state">

export type TurnCompletionDelayInput = {
  readonly sessionHasPromptWork: boolean
  readonly pendingTerminalRecordCount: number
  readonly pendingTerminalRecordFlush: boolean
  readonly lastTurnActivityAt: number
  readonly now: number
  readonly quietWindowMs: number
}

export function sessionFocusedAgentId<TAgent extends { id: string }>(
  session: {
    readonly agents: readonly TAgent[]
    readonly focused_agent_id?: string | null
  },
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

export function resolveVisibleTranscriptAgentId(
  splitMode: boolean,
  primaryAgentId: string | null,
  focusedAgentId: string | null,
): string | null {
  return splitMode ? (primaryAgentId ?? focusedAgentId) : focusedAgentId
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
  readonly toolActivityLabel?: (tool?: string | null) => string | null
}): string | null {
  const agentId = options.agentId
  if (!agentId) {
    return null
  }
  if (agentId === options.visibleTranscriptAgentId) {
    return Array.from(options.activeToolLabels).at(-1) ?? null
  }
  const toolActivityLabel = options.toolActivityLabel ?? getToolActivityLabel
  const labels = Array.from(options.agentPaneToolUpdates ?? [])
    .filter((update) => update.status !== "completed" && update.status !== "error" && update.status !== "cancelled")
    .map((update) => toolActivityLabel(update.tool))
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

import type {
  AgentInstance,
  RuntimeSession,
} from "./kernel-types.js"
import {
  agentLegacyProcessingStateIsBusy,
} from "./agent-activity.js"
import {
  sessionAllowsLegacyAgentProcessingState,
  sessionAgentIsBusy,
  sessionHasPromptWork,
  sessionProjectedStreamingAgentId,
} from "./session-prompt-work.js"
import {
  sessionHasAgentActivityProjection,
  sessionHasPromptStateProjection,
} from "./session-agent-prompt-state.js"
import {
  sessionPromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
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

export type SessionAuthoritativeIdleTransitionInput = {
  readonly nextSession: RuntimeSession
  readonly currentStatusLine?: string | null
  readonly cancellationRequestedStatusLine?: string
}

export type SessionAuthoritativeIdleTransitionState = {
  readonly shouldClearRuntimeResidue: boolean
  readonly shouldResetCancellationStatusLine: boolean
}

export type SessionRuntimeTransitionOptions = {
  readonly currentSession: RuntimeSession
  readonly nextSession: RuntimeSession
  readonly currentWorking: boolean
  readonly currentSubmitting?: boolean
  readonly currentBusyLatches?: Record<string, boolean>
  readonly currentStreamingAgentId: string | null
  readonly currentProviderActivityLabel?: string | null
  readonly currentActiveStatusLabel?: string | null
  readonly currentAgentActivityLabels: Record<string, string | null>
}

export type SessionRuntimeTransitionState = {
  readonly nextFocusedAgentId: string | null
  readonly nextHasPromptWork: boolean
  readonly nextStreamingAgentId: string | null
  readonly nextFocusedActivityLabel: string | null
  readonly nextAgentActivityLabels: Record<string, string | null>
  readonly nextWorking: boolean
  readonly activePromptChanged: boolean
  readonly cancelledPromptSettled: boolean
  readonly settledAgentIds: string[]
  readonly shouldClearWorkingAfterPromptSettlement: boolean
  readonly shouldClearCancelledPromptRuntimeResidue: boolean
  readonly shouldConfirmTurnCompletionAfterCancelledPromptSettlement: boolean
  readonly nextStreamingAgentIdAfterCancelledPromptSettlement: string | null
  readonly shouldConfirmIdleTurnCompletion: boolean
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
  readonly session: RuntimeSession
  readonly pendingTerminalRecordCount: number
  readonly pendingTerminalRecordFlush: boolean
  readonly lastTurnActivityAt: number
  readonly now: number
  readonly quietWindowMs: number
}

export type ProviderActivityRuntimeTransition = {
  readonly providerActivityActive: boolean
  readonly working: boolean | null
  readonly shouldUpdateSessionChrome: boolean
}

export type TurnCompletionProviderActivityTransition = {
  readonly shouldCancelPendingCompletion: boolean
  readonly shouldScheduleConfirmedCompletion: boolean
}

export const DEFAULT_CANCELLATION_REQUESTED_STATUS_LINE = "Cancellation requested."

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
    ? agents.find(agentLegacyProcessingStateIsBusy)?.id ?? null
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
  const shouldConfirmIdleTurnCompletion = sessionShouldConfirmIdleTurnCompletion({
    nextSession: options.nextSession,
    currentWorking: options.currentWorking,
    currentSubmitting: options.currentSubmitting ?? false,
    currentBusyLatches: options.currentBusyLatches ?? {},
    currentStreamingAgentId: options.currentStreamingAgentId,
    currentProviderActivityLabel: options.currentProviderActivityLabel ?? null,
    currentActiveStatusLabel: options.currentActiveStatusLabel ?? null,
  })
  const promptLifecycle = sessionPromptLifecycleTransition(options.currentSession, options.nextSession)
  const projectedStreamingAgentId = sessionProjectedStreamingAgentId(options.nextSession)
  const nextHasAgentActivityProjection = sessionHasAgentActivityProjection(options.nextSession)
  const nextHasPromptStateProjection = sessionHasPromptStateProjection(options.nextSession)
  const nextAllowsLegacyAgentProcessingState = sessionAllowsLegacyAgentProcessingState(options.nextSession)
  const nextStreamingAgentId = nextHasAgentActivityProjection
    ? projectedStreamingAgentId
    : resolveSessionStreamingAgentId(
      options.nextSession.agents,
      projectedStreamingAgentId,
      nextHasPromptWork,
      options.currentWorking,
      options.currentStreamingAgentId,
      !nextHasPromptStateProjection,
    )
  const nextAgentActivityLabels: Record<string, string | null> = {}
  for (const agent of options.nextSession.agents) {
    const legacyAgentBusy = nextAllowsLegacyAgentProcessingState
      && agentLegacyProcessingStateIsBusy(agent)
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
    nextWorking: sessionWorkingStateAfterPromptWork({
      currentWorking: options.currentWorking,
      nextSession: options.nextSession,
    }),
    activePromptChanged: promptLifecycle.activePromptChanged,
    cancelledPromptSettled: promptLifecycle.cancelledPromptSettled,
    settledAgentIds: promptLifecycle.settledAgentIds,
    shouldClearWorkingAfterPromptSettlement:
      promptLifecycle.settledAgentIds.length > 0 && !nextHasPromptWork,
    shouldClearCancelledPromptRuntimeResidue: promptLifecycle.cancelledPromptSettled,
    shouldConfirmTurnCompletionAfterCancelledPromptSettlement:
      promptLifecycle.cancelledPromptSettled && !nextHasPromptWork,
    nextStreamingAgentIdAfterCancelledPromptSettlement: projectedStreamingAgentId,
    shouldConfirmIdleTurnCompletion,
    previousAgentSignature,
    nextAgentSignature,
  }
}

export function sessionWorkingStateAfterPromptWork(options: {
  readonly currentWorking: boolean
  readonly nextSession: RuntimeSession
}): boolean {
  return sessionHasPromptWork(options.nextSession) ? true : options.currentWorking
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
    || (sessionAllowsLegacyAgentProcessingState(options.session)
      && options.session.agents.some((agent) => agent.id === agentId && agentLegacyProcessingStateIsBusy(agent)))
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
  const allowLegacyProcessing = sessionAllowsLegacyAgentProcessingState(options.session)
  return (options.submitting && options.submittingAgentId === agentId)
    || sessionAgentIsBusy(options.session, agentId)
    || options.streamingAgentId === agentId
    || Boolean(options.focusedActivityLabel)
    || readAgentBusyLatch(options.agentBusyLatches, agentId)
    || Boolean(allowLegacyProcessing && agentLegacyProcessingStateIsBusy(focused))
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
    const allowLegacyProcessing = sessionAllowsLegacyAgentProcessingState(options.session)
    const isBusy = (options.submitting && options.submittingAgentId === agentId)
      || sessionAgentIsBusy(options.session, agentId)
      || options.streamingAgentId === agentId
      || Boolean(options.agentActivityLabels[agentId])
      || readAgentBusyLatch(options.agentBusyLatches, agentId)
      || (allowLegacyProcessing && agentLegacyProcessingStateIsBusy(agent))
    return { id: agentId, busy: isBusy }
  })
}

export function sessionShouldConfirmIdleTurnCompletion(options: SessionIdleTurnCompletionInput): boolean {
  if (sessionHasPromptWork(options.nextSession)) {
    return false
  }
  return options.currentWorking
    || options.currentSubmitting
    || Object.values(options.currentBusyLatches).some(Boolean)
    || options.currentStreamingAgentId !== null
    || options.currentProviderActivityLabel !== null
    || options.currentActiveStatusLabel !== null
}

export function sessionAuthoritativeIdleTransitionState(
  options: SessionAuthoritativeIdleTransitionInput,
): SessionAuthoritativeIdleTransitionState {
  const shouldClearRuntimeResidue = !sessionHasPromptWork(options.nextSession)
  const cancellationRequestedStatusLine =
    options.cancellationRequestedStatusLine ?? DEFAULT_CANCELLATION_REQUESTED_STATUS_LINE
  return {
    shouldClearRuntimeResidue,
    shouldResetCancellationStatusLine:
      shouldClearRuntimeResidue && options.currentStatusLine === cancellationRequestedStatusLine,
  }
}

export function turnCompletionDelayMs(options: TurnCompletionDelayInput): number | null {
  if (
    sessionHasPromptWork(options.session)
    || options.pendingTerminalRecordCount > 0
    || options.pendingTerminalRecordFlush
  ) {
    return null
  }
  return Math.max(0, options.quietWindowMs - Math.max(0, options.now - options.lastTurnActivityAt))
}

export function providerActivityRuntimeTransition(active: boolean): ProviderActivityRuntimeTransition {
  return {
    providerActivityActive: active,
    working: active ? true : null,
    shouldUpdateSessionChrome: true,
  }
}

export function turnCompletionProviderActivityTransition(
  active: boolean,
): TurnCompletionProviderActivityTransition {
  return {
    shouldCancelPendingCompletion: active,
    shouldScheduleConfirmedCompletion: !active,
  }
}

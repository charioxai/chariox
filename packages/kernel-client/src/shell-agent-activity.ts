import type {
  AgentInstance,
  AgentPromptState,
  PromptQueueItem,
  RuntimeInteraction,
  RuntimeProviderRun,
  RuntimeSession,
} from "./kernel-types.js"
import {
  agentRuntimeActiveTurnIsBusy,
  agentRuntimeActivityIsBusy,
  agentRuntimePromptStatusIsActivePrompt,
  normalizeAgentRuntimeActivityStatus,
  normalizeAgentRuntimePromptStatus,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import type { AgentRuntimeActivityBusyInput } from "./agent-activity.js"

export type SessionPromptWorkSummary = {
  readonly active: number
  readonly queued: number
  readonly busyAgents: number
}

export type ActivePromptLifecycleRecord = {
  readonly id: string
  readonly status?: string
  readonly promptOrigin?: string | null
  readonly target_agent_id?: string | null
}

export type PromptLifecycleTransition = {
  readonly activePromptChanged: boolean
  readonly cancelledPromptSettled: boolean
  readonly settledAgentIds: string[]
}

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

export function sessionHasAgentRuntimeProjection(session: RuntimeSession | null | undefined): boolean {
  return Boolean(session?.agent_activity || session?.prompt_states)
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

export function sessionPromptWorkSummary(session: RuntimeSession): SessionPromptWorkSummary {
  const promptStates = session.prompt_states
  const promptStateEntries = promptStateEntriesForSessionAgents(session)
  const queued = promptStates
    ? promptStateEntries.reduce((count, [, state]) => count + (state?.queued_prompts?.length ?? 0), 0)
    : session.queued_prompts.length

  if (session.agent_activity) {
    const activities = Object.entries(session.agent_activity)
    const projectedQueued = promptWorkCountFromProjectedActivities(
      activities.map(([, activity]) => activity),
      "queued_prompt_count",
    )
    return {
      active: activities.reduce(
        (count, [agentId, activity]) =>
          count + projectedActivePromptCount(activity, promptStates?.[agentId]),
        0,
      ),
      queued: projectedQueued ?? queued,
      busyAgents: activities.filter(([, activity]) => agentRuntimeActivityIsBusy(activity)).length,
    }
  }

  if (promptStates) {
    const busyAgents = promptStateEntries.filter(([, state]) => promptStateHasWork(state)).length
    return {
      active: promptStateEntries.filter(([, state]) => Boolean(state?.active_prompt)).length,
      queued,
      busyAgents,
    }
  }

  return {
    active: session.active_prompt ? 1 : 0,
    queued,
    busyAgents: legacyBusyAgentCount(session),
  }
}

export function sessionAgentIsBusy(session: RuntimeSession | null | undefined, agentId: string | null | undefined): boolean {
  if (!session || !agentId) {
    return false
  }
  if (!sessionHasAgent(session, agentId)) {
    return false
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    return agentRuntimeActivityIsBusy(projected)
  }
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return Boolean(promptState?.active_prompt) || Boolean(promptState?.queued_prompts?.length)
  }
  return legacyTopLevelSessionHasPromptWork(session, agentId)
}

export function sessionHasPromptWork(session: RuntimeSession): boolean {
  const summary = sessionPromptWorkSummary(session)
  return summary.active > 0 || summary.queued > 0 || summary.busyAgents > 0
}

export function sessionHasProcessingAgent(session: RuntimeSession): boolean {
  return sessionPromptWorkSummary(session).busyAgents > 0
}

export function sessionPromptWorkByAgent(session: RuntimeSession): Record<string, boolean> {
  const state: Record<string, boolean> = {}
  for (const agent of session.agents) {
    state[agent.id] = sessionAgentIsBusy(session, agent.id)
  }
  return state
}

export function sessionProjectedStreamingAgentId(session: RuntimeSession): string | null {
  if (session.agent_activity) {
    return session.agents.find((agent) => agentRuntimeActivityIsBusy(session.agent_activity?.[agent.id]))?.id ?? null
  }
  if (session.prompt_states) {
    const activeAgents = session.agents.filter((agent) => {
      const promptState = session.prompt_states?.[agent.id]
      return Boolean(promptState?.active_prompt)
    })
    return activeAgents.length === 1 ? activeAgents[0]?.id ?? null : null
  }
  return session.active_prompt?.target_agent_id ?? null
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
    if (normalizeAgentRuntimeActivityStatus(projected.status) === "error") {
      return "Error"
    }
    return agentRuntimeActivityIsBusy(projected) ? "Working" : "Idle"
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
  return activity?.unread_idle_output === true
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
    const activeTurn = projected.active_turn
    if (agentRuntimeActiveTurnIsBusy(activeTurn)) {
      return activeTurn?.prompt_id === promptId
    }
    if (!agentRuntimeActivityIsBusy(projected)) {
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
    const activeTurnPromptId = agentRuntimeActiveTurnIsBusy(projected.active_turn)
      ? projected.active_turn?.prompt_id
      : null
    if (activeTurnPromptId) {
      const prompt = legacyPromptForAgent(session, agentId)
      return prompt?.id === activeTurnPromptId ? prompt : null
    }
    if (!agentRuntimeActivityIsBusy(projected)) {
      return null
    }
  }
  return legacyPromptForAgent(session, agentId)
}

export function sessionActivePromptIdForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): string | null {
  if (agentId) {
    const projected = session.agent_activity?.[agentId]
    const projectedPromptId = agentRuntimeActiveTurnIsBusy(projected?.active_turn)
      ? projected?.active_turn?.prompt_id
      : null
    if (projectedPromptId) {
      return projectedPromptId
    }
    if (session.agent_activity && !agentRuntimeActivityIsBusy(projected)) {
      return null
    }
    return activePromptForAgent(session, agentId)?.id ?? null
  }

  const records = sessionActivePromptLifecycleRecords(session)
  return records.length === 1 ? records[0]?.id ?? null : null
}

export function sessionActivePromptLifecycleRecords(session: RuntimeSession): ActivePromptLifecycleRecord[] {
  if (session.agent_activity) {
    const records: ActivePromptLifecycleRecord[] = []
    for (const [agentId, activity] of Object.entries(session.agent_activity)) {
      const activeTurn = activity.active_turn
      if (activeTurn && agentRuntimeActiveTurnIsBusy(activeTurn)) {
        records.push({
          id: activeTurn.prompt_id,
          status: activeTurn.status,
          promptOrigin: activeTurn.prompt_origin ?? null,
          target_agent_id: agentId,
        })
        continue
      }
      if (!agentRuntimeActivityIsBusy(activity)) {
        continue
      }
      const stateActivePrompt = session.prompt_states?.[agentId]?.active_prompt
      if (stateActivePrompt) {
        records.push(activePromptLifecycleRecordFromPrompt(stateActivePrompt))
      }
    }
    return records.sort(compareActivePromptLifecycleRecords)
  }
  if (session.prompt_states) {
    return Object.values(session.prompt_states)
      .map((state) => state.active_prompt)
      .map((stateActivePrompt) => stateActivePrompt
        ? activePromptLifecycleRecordFromPrompt(stateActivePrompt)
        : null)
      .filter((prompt): prompt is ActivePromptLifecycleRecord => Boolean(prompt))
      .sort(compareActivePromptLifecycleRecords)
  }
  return session.active_prompt
    ? [activePromptLifecycleRecordFromPrompt(session.active_prompt)]
    : []
}

export function sessionPromptLifecycleTransition(
  currentSession: RuntimeSession,
  nextSession: RuntimeSession,
): PromptLifecycleTransition {
  const currentPromptRecords = sessionActivePromptLifecycleRecords(currentSession)
  const previousPromptIds = currentPromptRecords.map((prompt) => prompt.id)
  const nextPromptIds = activePromptLifecycleRecordIds(nextSession)
  const nextPromptIdSet = new Set(nextPromptIds)
  const settledPromptRecords = currentPromptRecords
    .filter((prompt) => !nextPromptIdSet.has(prompt.id))

  return {
    activePromptChanged:
      previousPromptIds.length !== nextPromptIds.length
      || previousPromptIds.some((id, index) => id !== nextPromptIds[index]),
    settledAgentIds: settledPromptRecords
      .map((prompt) => prompt.target_agent_id)
      .filter((agentId): agentId is string => Boolean(agentId)),
    cancelledPromptSettled:
      currentPromptRecords.some((prompt) => prompt.status === "cancelling" && !nextPromptIdSet.has(prompt.id)),
  }
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

function activePromptLifecycleRecordFromPrompt(prompt: PromptQueueItem): ActivePromptLifecycleRecord {
  return {
    ...prompt,
    promptOrigin: prompt.prompt_origin ?? null,
  }
}

function activePromptLifecycleRecordIds(session: RuntimeSession): string[] {
  return sessionActivePromptLifecycleRecords(session).map((prompt) => prompt.id)
}

function compareActivePromptLifecycleRecords(
  left: ActivePromptLifecycleRecord,
  right: ActivePromptLifecycleRecord,
): number {
  return left.id.localeCompare(right.id)
}

function promptStateHasWork(state: AgentPromptStateLike | null | undefined): boolean {
  return Boolean(state?.active_prompt) || Boolean(state?.queued_prompts?.length)
}

function promptStateEntriesForSessionAgents(
  session: RuntimeSession,
): readonly (readonly [string, AgentPromptStateLike | null | undefined])[] {
  const promptStates = session.prompt_states
  if (!promptStates) {
    return []
  }
  const agentIds = sessionAgentIds(session)
  return Object.entries(promptStates).filter(([agentId]) => agentIds.has(agentId))
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

function legacyTopLevelSessionHasPromptWork(session: RuntimeSession, agentId: string): boolean {
  return Boolean(session.active_prompt?.target_agent_id === agentId)
    || Boolean(session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId))
}

function agentRuntimeActivityHasActivePrompt(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string],
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): boolean {
  const projection = projectAgentRuntimeActivity(activity)
  if (projection.activeTurn) {
    return projection.activePromptCount > 0
  }
  if (agentRuntimeActivityIsBusy(activity) && promptState?.active_prompt) {
    return true
  }
  return agentRuntimePromptStatusIsActivePrompt(projection.promptStatus)
}

function projectedActivePromptCount(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string],
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): number {
  const projection = projectAgentRuntimeActivity(activity)
  if (activity.active_prompt_count !== undefined && activity.active_prompt_count !== null) {
    return projection.activePromptCount
  }
  return agentRuntimeActivityHasActivePrompt(activity, promptState) ? 1 : 0
}

function promptWorkCountFromProjectedActivities(
  activities: readonly NonNullable<RuntimeSession["agent_activity"]>[string][],
  field: "queued_prompt_count",
): number | null {
  let count = 0
  for (const activity of activities) {
    const projectedCount = nonNegativeInteger(activity[field])
    if (projectedCount === null) {
      return null
    }
    count += projectedCount
  }
  return count
}

function nonNegativeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : null
}

function legacyBusyAgentCount(session: RuntimeSession): number {
  return session.agents.filter((agent) => agent.is_processing || agent.state === "Working").length
}

function legacyAgentRuntimeState(agent: AgentInstance): AgentInstance["state"] {
  return agent.is_processing && agent.state !== "Error" ? "Working" : agent.state
}

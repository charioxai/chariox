import process from "node:process"

import {
  agentRuntimeActivityIsBusy as kernelAgentRuntimeActivityIsBusy,
} from "@arroba/kernel-client/agent-activity"
import {
  normalizeAgentPromptState,
  type AgentPromptState,
  type CliOptions,
  type RuntimeInteraction,
  type RuntimeProviderRun,
  type RuntimeSession,
  type SessionHistoryCursorState,
  type TranscriptEntry,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { reconcileWorkingStateFromSession, resolveStreamingAgentId } from "./runtime.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export const NO_SESSION_ID = "no-session"
export const SESSION_CONFIG_RESPONSE_LAYOUT_KEY = "ui.multiAgentResponseLayout"

type SessionTransitionOptions = {
  currentSession: RuntimeSession
  nextSession: RuntimeSession
  currentWorking: boolean
  currentStreamingAgentId: string | null
  currentAgentActivityLabels: Record<string, string | null>
  layoutPreference?: MultiAgentResponseLayout | null | undefined
}

export type SessionTransitionState = {
  nextFocusedAgentId: string | null
  nextHasPromptWork: boolean
  nextStreamingAgentId: string | null
  nextFocusedActivityLabel: string | null
  nextAgentActivityLabels: Record<string, string | null>
  nextLayout: MultiAgentResponseLayout
  nextWorking: boolean
  previousAgentSignature: string
  nextAgentSignature: string
}

export type PromptLifecycleTransition = {
  activePromptChanged: boolean
  cancelledPromptSettled: boolean
  settledAgentIds: string[]
}

export type DetachedCliTransitionState = {
  centerMode: "transcript"
  createdSession: false
  session: RuntimeSession
  providerActivityLabel: null
  activeStatusLabel: null
  agentPaneEntries: Record<string, TranscriptEntry[]>
  agentPanePreviews: Record<string, string>
  agentActivityLabels: Record<string, string | null>
  streamingAgentId: null
  submitting: false
  working: false
  fatalError: null
  daemonDisconnected: false
  nextHistoryCursor: SessionHistoryCursorState
  statusLine: string
  waitingRoomState: WaitingRoomState
}

export type AttachedCliTransitionState = {
  centerMode: "transcript"
  createdSession: boolean
  session: RuntimeSession
  providerActivityLabel: null
  activeStatusLabel: null
  fatalError: null
  daemonDisconnected: false
  submitting: false
  working: boolean
  statusLine: string
}

export function buildDetachedSessionState(options: CliOptions): RuntimeSession {
  const workspace = options.workspace ?? process.cwd()
  const worktree = options.worktree ?? workspace
  return {
    id: NO_SESSION_ID,
    alias: null,
    workspace_id: workspace,
    worktree_id: worktree,
    created_at_ms: Date.now(),
    status: "Parked",
    agent_defaults: {
      provider: options.provider ?? "opencode",
      model: options.model,
      effort: options.effort,
      account_profile: options.accountProfile,
      execution_mode: "build",
      permission_level: "yolo",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
  }
}

export function sessionHasPromptWork(session: RuntimeSession): boolean {
  if (session.agent_activity) {
    return Object.values(session.agent_activity).some(agentRuntimeActivityIsBusy)
  }
  if (session.prompt_states) {
    return Object.values(session.prompt_states).some((state) => {
      return Boolean(state.active_prompt) || state.queued_prompts.length > 0
    })
  }
  return Boolean(session.active_prompt) || session.queued_prompts.length > 0
}

export function sessionHasProcessingAgent(session: RuntimeSession): boolean {
  if (session.agent_activity) {
    return Object.values(session.agent_activity).some(agentRuntimeActivityIsBusy)
  }
  if (session.prompt_states) {
    return Object.values(session.prompt_states).some((state) => {
      return Boolean(state.active_prompt) || state.queued_prompts.length > 0
    })
  }
  return session.agents.some((agent) => {
    return agent.is_processing || agent.state === "Working"
  })
}

export function focusedAgentIdForSession(session: RuntimeSession): string | null {
  const focusedAgentId = session.focused_agent_id
  if (focusedAgentId && session.agents.some((agent) => agent.id === focusedAgentId)) {
    return focusedAgentId
  }
  if (focusedAgentId) {
    return null
  }
  return session.agents[0]?.id ?? null
}

export function activeInteractionForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): RuntimeInteraction | null {
  if (!agentId) {
    return null
  }
  return session.active_interactions?.find((interaction) => interaction.agent_id === agentId) ?? null
}

export function focusedProviderRunForAgent(
  run: RuntimeProviderRun | null,
  focusedAgentId: string | null | undefined,
): RuntimeProviderRun | null {
  return run && run.agent_instance_id === focusedAgentId ? run : null
}

export function activePromptIdForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): string | null {
  if (agentId) {
    const projectedActivity = session.agent_activity?.[agentId]
    const projectedPromptId = projectedActivity?.active_turn?.prompt_id
    if (projectedPromptId) {
      return projectedPromptId
    }
    if (session.agent_activity && !agentRuntimeActivityIsBusy(projectedActivity)) {
      return null
    }
    return agentPromptState(session, agentId)?.active_prompt?.id ?? null
  }

  const activePromptRecords = collectActivePromptRecords(session)
  return activePromptRecords.length === 1 ? activePromptRecords[0]?.id ?? null : null
}

export function shouldConfirmIdleTurnCompletion(options: {
  nextSession: RuntimeSession
  currentWorking: boolean
  currentSubmitting: boolean
  currentBusyLatches: Record<string, boolean>
  currentStreamingAgentId: string | null
  currentProviderActivityLabel: string | null
  currentActiveStatusLabel: string | null
}): boolean {
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

export function agentPromptState(
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
  if (session.active_prompt?.target_agent_id === agentId || session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId)) {
    return {
      active_prompt: session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null,
      queued_prompts: session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId),
    }
  }
  return null
}

export function agentHasPromptWork(
  session: RuntimeSession,
  agentId: string | null | undefined,
): boolean {
  if (session.agent_activity) {
    return agentId ? agentRuntimeActivityIsBusy(session.agent_activity[agentId]) : false
  }
  const promptState = agentPromptState(session, agentId)
  return Boolean(promptState?.active_prompt) || (promptState?.queued_prompts.length ?? 0) > 0
}

export function promptWorkByAgent(session: RuntimeSession): Record<string, boolean> {
  const state: Record<string, boolean> = {}
  for (const agent of session.agents) {
    state[agent.id] = agentHasPromptWork(session, agent.id)
  }
  return state
}

export function sessionResponseLayout(
  session: RuntimeSession | null | undefined,
  fallback?: MultiAgentResponseLayout | null,
): MultiAgentResponseLayout {
  return normalizeMultiAgentResponseLayout(
    session?.config_state?.values?.[SESSION_CONFIG_RESPONSE_LAYOUT_KEY],
  )
    ?? normalizeMultiAgentResponseLayout(fallback)
    ?? "individual"
}

export function projectedStreamingAgentIdForSession(session: RuntimeSession): string | null {
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

export function agentRuntimeActivityIsBusy(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string] | null | undefined,
): boolean {
  return kernelAgentRuntimeActivityIsBusy(activity)
}

export function deriveSessionTransitionState(
  options: SessionTransitionOptions,
): SessionTransitionState {
  const previousAgentSignature = options.currentSession.agents
    .map((agent) => agent.id)
    .join(",")
  const nextAgentSignature = options.nextSession.agents.map((agent) => agent.id).join(",")
  const nextFocusedAgentId = focusedAgentIdForSession(options.nextSession)
  const nextHasPromptWork = sessionHasPromptWork(options.nextSession)
  const resolvedStreamingAgentId = options.nextSession.agent_activity
    ? projectedStreamingAgentIdForSession(options.nextSession)
    : resolveStreamingAgentId(
      options.nextSession.agents,
      projectedStreamingAgentIdForSession(options.nextSession),
      nextHasPromptWork,
      options.currentWorking,
      options.currentStreamingAgentId,
      !options.nextSession.prompt_states,
    )
  const nextStreamingAgentId = resolvedStreamingAgentId
  const nextAgentActivityLabels: Record<string, string | null> = {}
  for (const agent of options.nextSession.agents) {
    const legacyAgentBusy = !options.nextSession.agent_activity && !options.nextSession.prompt_states && (
      agent.is_processing
      || agent.state === "Working"
    )
    nextAgentActivityLabels[agent.id] =
      legacyAgentBusy
        || agent.id === nextStreamingAgentId
        || agentHasPromptWork(options.nextSession, agent.id)
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
    nextLayout: sessionResponseLayout(options.nextSession, options.layoutPreference),
    nextWorking: reconcileWorkingStateFromSession(
      options.currentWorking,
      nextHasPromptWork,
    ),
    previousAgentSignature,
    nextAgentSignature,
  }
}

export function derivePromptLifecycleTransition(
  currentSession: RuntimeSession,
  nextSession: RuntimeSession,
): PromptLifecycleTransition {
  const previousPromptIds = collectActivePromptIds(currentSession)
  const nextPromptIds = collectActivePromptIds(nextSession)
  const nextPromptIdSet = new Set(nextPromptIds)
  const settledPromptRecords = collectActivePromptRecords(currentSession)
    .filter((prompt) => !nextPromptIdSet.has(prompt.id))

  return {
    activePromptChanged:
      previousPromptIds.length !== nextPromptIds.length
      || previousPromptIds.some((id, index) => id !== nextPromptIds[index]),
    settledAgentIds: settledPromptRecords
      .map((prompt) => prompt.target_agent_id)
      .filter((agentId): agentId is string => Boolean(agentId)),
    cancelledPromptSettled:
      collectActivePromptRecords(currentSession)
        .some((prompt) => prompt.status === "cancelling" && !nextPromptIdSet.has(prompt.id)),
  }
}

export function deriveDetachedCliTransitionState(options: {
  cliOptions: CliOptions
  waitingRoomState: WaitingRoomState
  message: string
}): DetachedCliTransitionState {
  return {
    centerMode: "transcript",
    createdSession: false,
    session: buildDetachedSessionState(options.cliOptions),
    providerActivityLabel: null,
    activeStatusLabel: null,
    agentPaneEntries: {},
    agentPanePreviews: {},
    agentActivityLabels: {},
    streamingAgentId: null,
    submitting: false,
    working: false,
    fatalError: null,
    daemonDisconnected: false,
    nextHistoryCursor: null,
    statusLine: options.message,
    waitingRoomState: resetWaitingRoomState(options.waitingRoomState),
  }
}

export function deriveAttachedCliTransitionState(options: {
  session: RuntimeSession
  createdSession: boolean
  connectedStatus: string
}): AttachedCliTransitionState {
  return {
    centerMode: "transcript",
    createdSession: options.createdSession,
    session: options.session,
    providerActivityLabel: null,
    activeStatusLabel: null,
    fatalError: null,
    daemonDisconnected: false,
    submitting: false,
    working: sessionHasPromptWork(options.session),
    statusLine: options.connectedStatus,
  }
}

function resetWaitingRoomState(state: WaitingRoomState): WaitingRoomState {
  return {
    ...state,
    focus: "new",
    machineIndex: 0,
    remoteKernelIndex: 0,
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
  }
}

function normalizeMultiAgentResponseLayout(
  value?: string | null,
): MultiAgentResponseLayout | null {
  return value === "split" || value === "individual" ? value : null
}

type ActivePromptLifecycleRecord = {
  id: string
  status?: string
  promptOrigin?: string | null
  target_agent_id?: string | null
}

function collectActivePromptRecords(session: RuntimeSession): ActivePromptLifecycleRecord[] {
  if (session.agent_activity) {
    const records: ActivePromptLifecycleRecord[] = []
    for (const [agentId, activity] of Object.entries(session.agent_activity)) {
      const activeTurn = activity.active_turn
      if (!activeTurn) {
        continue
      }
      records.push({
        id: activeTurn.prompt_id,
        status: activeTurn.status,
        promptOrigin: activeTurn.prompt_origin ?? null,
        target_agent_id: agentId,
      })
    }
    return records.sort((left, right) => left.id.localeCompare(right.id))
  }
  if (session.prompt_states) {
    return Object.values(session.prompt_states)
      .map((state) => state.active_prompt)
      .map((stateActivePrompt) => stateActivePrompt ? {
        ...stateActivePrompt,
        promptOrigin: stateActivePrompt.prompt_origin ?? null,
      } : null)
      .filter((prompt): prompt is NonNullable<typeof prompt> => Boolean(prompt))
      .sort((left, right) => left.id.localeCompare(right.id))
  }
  return session.active_prompt ? [{
    ...session.active_prompt,
    promptOrigin: session.active_prompt.prompt_origin ?? null,
  }] : []
}

function collectActivePromptIds(session: RuntimeSession) {
  return collectActivePromptRecords(session).map((prompt) => prompt.id)
}

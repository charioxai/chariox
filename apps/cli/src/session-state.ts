import process from "node:process"

import {
  normalizeAgentPromptState,
  type AgentPromptState,
  type CliOptions,
  type RuntimeSession,
  type SessionHistoryCursor,
  type TranscriptEntry,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { reconcileWorkingStateFromSession, resolveStreamingAgentId } from "./runtime.js"
import type { WaitingRoomState } from "./waiting-room.js"

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
  nextHistoryCursor: SessionHistoryCursor | null
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
  if (session.prompt_states && Object.keys(session.prompt_states).length > 0) {
    return Object.values(session.prompt_states).some((state) => {
      return Boolean(state.active_prompt) || state.queued_prompts.length > 0
    })
  }
  return Boolean(session.active_prompt) || session.queued_prompts.length > 0
}

export function sessionHasProcessingAgent(session: RuntimeSession): boolean {
  return session.agents.some((agent) => {
    return agent.is_processing || agent.state === "Working"
  })
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
  const promptState = session.prompt_states?.[agentId]
  if (promptState) {
    return normalizeAgentPromptState(promptState)
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
  const promptState = agentPromptState(session, agentId)
  return Boolean(promptState?.active_prompt) || (promptState?.queued_prompts.length ?? 0) > 0
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

export function deriveSessionTransitionState(
  options: SessionTransitionOptions,
): SessionTransitionState {
  const previousAgentSignature = options.currentSession.agents
    .map((agent) => agent.id)
    .join(",")
  const nextAgentSignature = options.nextSession.agents.map((agent) => agent.id).join(",")
  const nextFocusedAgentId =
    options.nextSession.focused_agent_id ?? options.nextSession.agents[0]?.id ?? null
  const nextHasPromptWork = sessionHasPromptWork(options.nextSession)
  const resolvedStreamingAgentId = resolveStreamingAgentId(
    options.nextSession.agents,
    options.nextSession.active_prompt?.target_agent_id ?? null,
    nextHasPromptWork,
    options.currentWorking,
    options.currentStreamingAgentId,
  )
  const nextStreamingAgentId = resolvedStreamingAgentId
  const nextAgentActivityLabels: Record<string, string | null> = {}
  for (const agent of options.nextSession.agents) {
    nextAgentActivityLabels[agent.id] =
      agent.is_processing
        || agent.state === "Working"
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
  return {
    activePromptChanged:
      previousPromptIds.length !== nextPromptIds.length
      || previousPromptIds.some((id, index) => id !== nextPromptIds[index]),
    settledAgentIds: collectActivePromptRecords(currentSession)
      .filter((prompt) => !nextPromptIdSet.has(prompt.id))
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
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
  }
}

function normalizeMultiAgentResponseLayout(
  value?: string | null,
): MultiAgentResponseLayout | null {
  return value === "split" || value === "individual" ? value : null
}

function collectActivePromptRecords(session: RuntimeSession) {
  if (session.prompt_states && Object.keys(session.prompt_states).length > 0) {
    return Object.values(session.prompt_states)
      .map((state) => state.active_prompt)
      .filter((prompt): prompt is NonNullable<typeof prompt> => Boolean(prompt))
      .sort((left, right) => left.id.localeCompare(right.id))
  }
  return session.active_prompt ? [session.active_prompt] : []
}

function collectActivePromptIds(session: RuntimeSession) {
  return collectActivePromptRecords(session).map((prompt) => prompt.id)
}

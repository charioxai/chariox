import process from "node:process"

import {
  agentRuntimeActivityIsBusy as kernelAgentRuntimeActivityIsBusy,
} from "@arroba/kernel-client/agent-activity"
import {
  sessionResponseLayout as kernelSessionResponseLayout,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY as KERNEL_SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "@arroba/kernel-client/session-config-projection"
import {
  runtimeProviderRunForAgent as kernelRuntimeProviderRunForAgent,
  sessionActivePromptIdForAgent as kernelSessionActivePromptIdForAgent,
  sessionActiveInteractionForAgent as kernelSessionActiveInteractionForAgent,
  sessionAgentIsBusy as kernelSessionAgentIsBusy,
  sessionFocusedAgentId as kernelSessionFocusedAgentId,
  sessionHasProcessingAgent as kernelSessionHasProcessingAgent,
  sessionHasAgentRuntimeProjection as kernelSessionHasAgentRuntimeProjection,
  sessionHasPromptWork as kernelSessionHasPromptWork,
  sessionPromptLifecycleTransition as kernelSessionPromptLifecycleTransition,
  sessionProjectedStreamingAgentId as kernelSessionProjectedStreamingAgentId,
  sessionPromptStateForAgent as kernelSessionPromptStateForAgent,
  sessionPromptWorkByAgent as kernelSessionPromptWorkByAgent,
  sessionShouldConfirmIdleTurnCompletion as kernelSessionShouldConfirmIdleTurnCompletion,
  sessionRuntimeTransitionState as kernelSessionRuntimeTransitionState,
} from "@arroba/kernel-client/shell-agent-activity"
import type {
  PromptLifecycleTransition as KernelPromptLifecycleTransition,
} from "@arroba/kernel-client/shell-agent-activity"
import {
  type AgentPromptState,
  type CliOptions,
  type RuntimeInteraction,
  type RuntimeProviderRun,
  type RuntimeSession,
  type SessionHistoryCursorState,
  type TranscriptEntry,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export const NO_SESSION_ID = "no-session"
export const SESSION_CONFIG_RESPONSE_LAYOUT_KEY = KERNEL_SESSION_CONFIG_RESPONSE_LAYOUT_KEY

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

export type PromptLifecycleTransition = KernelPromptLifecycleTransition

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
    workflow_schedules: [],
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
  return kernelSessionHasPromptWork(session as Parameters<typeof kernelSessionHasPromptWork>[0])
}

export function sessionHasProjectedRuntimeState(session: RuntimeSession): boolean {
  return kernelSessionHasAgentRuntimeProjection(session as Parameters<typeof kernelSessionHasAgentRuntimeProjection>[0])
}

export function sessionHasProcessingAgent(session: RuntimeSession): boolean {
  return kernelSessionHasProcessingAgent(session as Parameters<typeof kernelSessionHasProcessingAgent>[0])
}

export function focusedAgentIdForSession(session: RuntimeSession): string | null {
  return kernelSessionFocusedAgentId(session as Parameters<typeof kernelSessionFocusedAgentId>[0])
}

export function activeInteractionForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): RuntimeInteraction | null {
  return kernelSessionActiveInteractionForAgent(
    session as Parameters<typeof kernelSessionActiveInteractionForAgent>[0],
    agentId,
  ) as RuntimeInteraction | null
}

export function focusedProviderRunForAgent(
  run: RuntimeProviderRun | null,
  focusedAgentId: string | null | undefined,
): RuntimeProviderRun | null {
  return kernelRuntimeProviderRunForAgent(
    run as Parameters<typeof kernelRuntimeProviderRunForAgent>[0],
    focusedAgentId,
  ) as RuntimeProviderRun | null
}

export function activePromptIdForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): string | null {
  return kernelSessionActivePromptIdForAgent(
    session as Parameters<typeof kernelSessionActivePromptIdForAgent>[0],
    agentId,
  )
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
  return kernelSessionShouldConfirmIdleTurnCompletion({
    ...options,
    nextSession: options.nextSession as Parameters<typeof kernelSessionShouldConfirmIdleTurnCompletion>[0]["nextSession"],
  })
}

export function agentPromptState(
  session: RuntimeSession,
  agentId: string | null | undefined,
): AgentPromptState | null {
  return kernelSessionPromptStateForAgent(
    session as Parameters<typeof kernelSessionPromptStateForAgent>[0],
    agentId,
  ) as AgentPromptState | null
}

export function agentHasPromptWork(
  session: RuntimeSession,
  agentId: string | null | undefined,
): boolean {
  return kernelSessionAgentIsBusy(session as Parameters<typeof kernelSessionAgentIsBusy>[0], agentId)
}

export function promptWorkByAgent(session: RuntimeSession): Record<string, boolean> {
  return kernelSessionPromptWorkByAgent(session as Parameters<typeof kernelSessionPromptWorkByAgent>[0])
}

export function sessionResponseLayout(
  session: RuntimeSession | null | undefined,
  fallback?: MultiAgentResponseLayout | null,
): MultiAgentResponseLayout {
  return kernelSessionResponseLayout(
    session as Parameters<typeof kernelSessionResponseLayout>[0],
    fallback,
  )
}

export function projectedStreamingAgentIdForSession(session: RuntimeSession): string | null {
  return kernelSessionProjectedStreamingAgentId(
    session as Parameters<typeof kernelSessionProjectedStreamingAgentId>[0],
  )
}

export function agentRuntimeActivityIsBusy(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string] | null | undefined,
): boolean {
  return kernelAgentRuntimeActivityIsBusy(activity)
}

export function deriveSessionTransitionState(
  options: SessionTransitionOptions,
): SessionTransitionState {
  const transition = kernelSessionRuntimeTransitionState({
    currentSession: options.currentSession as Parameters<typeof kernelSessionRuntimeTransitionState>[0]["currentSession"],
    nextSession: options.nextSession as Parameters<typeof kernelSessionRuntimeTransitionState>[0]["nextSession"],
    currentWorking: options.currentWorking,
    currentStreamingAgentId: options.currentStreamingAgentId,
    currentAgentActivityLabels: options.currentAgentActivityLabels,
  })

  return {
    ...transition,
    nextLayout: sessionResponseLayout(options.nextSession, options.layoutPreference),
  }
}

export function derivePromptLifecycleTransition(
  currentSession: RuntimeSession,
  nextSession: RuntimeSession,
): PromptLifecycleTransition {
  return kernelSessionPromptLifecycleTransition(
    currentSession as Parameters<typeof kernelSessionPromptLifecycleTransition>[0],
    nextSession as Parameters<typeof kernelSessionPromptLifecycleTransition>[1],
  )
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

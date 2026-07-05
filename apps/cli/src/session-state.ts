import process from "node:process"

import {
  sessionHasPromptWork as kernelSessionHasPromptWork,
} from "@arroba/kernel-client/session-prompt-work"
import {
  type CliOptions,
  type RuntimeSession,
  type SessionHistoryCursorState,
  type TranscriptEntry,
} from "./cli-types.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export const NO_SESSION_ID = "no-session"

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
    working: kernelSessionHasPromptWork(options.session as Parameters<typeof kernelSessionHasPromptWork>[0]),
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

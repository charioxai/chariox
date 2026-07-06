import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, CliOptions, RuntimeSession } from "./cli-types.js"
import {
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  buildDetachedSessionState,
} from "./session-state.js"

test("buildDetachedSessionState creates a parked local placeholder session", () => {
  const session = buildDetachedSessionState({
    clientId: "client-1",
    model: "default",
    accountProfile: "default",
    effort: "high",
    workspace: "/workspace",
    worktree: "/workspace/tree",
  } satisfies CliOptions)

  assert.equal(session.id, "no-session")
  assert.equal(session.status, "Parked")
  assert.equal(session.workspace_id, "/workspace")
  assert.equal(session.worktree_id, "/workspace/tree")
  assert.equal(session.active_prompt, null)
  assert.deepEqual(session.agents, [])
})

test("deriveDetachedCliTransitionState resets waiting room and clears session-bound state", () => {
  const detached = deriveDetachedCliTransitionState({
    cliOptions: {
      clientId: "client-1",
      provider: "opencode",
      model: "default",
      accountProfile: "default",
      effort: "high",
      workspace: "/workspace",
      worktree: "/workspace/tree",
    },
    waitingRoomState: {
      focus: "session",
      sessionIndex: 3,
      machineIndex: 0,
      remoteKernelIndex: 0,
      terminalIndex: 0,
      worktreeSelectionId: "existing:/workspace/tree",
      workspaceLiveSyncMode: "off",
      providerId: "opencode",
      modelId: "opencode/gpt-5.4",
      effort: "medium",
      themeId: "opencode",
      introStep: 8,
      keyState: { up: true, down: false, left: true, right: false },
    },
    message: "No session attached.",
  })

  assert.equal(detached.centerMode, "transcript")
  assert.equal(detached.createdSession, false)
  assert.equal(detached.session.id, "no-session")
  assert.equal(detached.statusLine, "No session attached.")
  assert.equal(detached.waitingRoomState.focus, "new")
  assert.equal(detached.waitingRoomState.introStep, 0)
  assert.deepEqual(detached.waitingRoomState.keyState, {
    up: false,
    down: false,
    left: false,
    right: false,
  })
  assert.deepEqual(detached.agentPaneEntries, {})
  assert.deepEqual(detached.agentActivityLabels, {})
})

test("deriveAttachedCliTransitionState resets transient UI state and keeps prompt work", () => {
  const attached = deriveAttachedCliTransitionState({
    session: session({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "hello",
        status: "running",
      },
      agents: [agent("agent-a", { is_processing: true, state: "Working" })],
    }),
    createdSession: true,
    connectedStatus: "",
  })

  assert.equal(attached.centerMode, "transcript")
  assert.equal(attached.createdSession, true)
  assert.equal(attached.providerActivityLabel, null)
  assert.equal(attached.activeStatusLabel, null)
  assert.equal(attached.submitting, false)
  assert.equal(attached.streamingAgentId, "agent-a")
  assert.equal(attached.working, true)
  assert.equal(attached.statusLine, "")
})

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "default",
    worktree_id: "/workspace/tree",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}

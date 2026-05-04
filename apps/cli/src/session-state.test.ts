import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, CliOptions, RuntimeSession } from "./cli-types.js"
import {
  agentPromptState,
  agentHasPromptWork,
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  buildDetachedSessionState,
  derivePromptLifecycleTransition,
  deriveSessionTransitionState,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionResponseLayout,
  shouldConfirmIdleTurnCompletion,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
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

test("sessionResponseLayout prefers session config over fallback", () => {
  const splitSession = session({
    config_state: {
      version: 1,
      values: {
        [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: "split",
      },
      updated_by_attachment_id: null,
    },
  })

  assert.equal(sessionResponseLayout(splitSession, "individual"), "split")
  assert.equal(sessionResponseLayout(session(), "split"), "split")
  assert.equal(sessionResponseLayout(session(), null), "individual")
})

test("deriveSessionTransitionState preserves active agent labels and clears idle ones", () => {
  const currentSession = session({
    focused_agent_id: "agent-a",
    agents: [agent("agent-a"), agent("agent-b")],
  })
  const nextSession = session({
    focused_agent_id: "agent-b",
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-b",
      prompt: "hello",
      status: "running",
    },
    agents: [
      agent("agent-a"),
      agent("agent-b", { is_processing: true, state: "Working" }),
    ],
    config_state: {
      version: 1,
      values: {
        [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: "split",
      },
      updated_by_attachment_id: null,
    },
  })

  const transition = deriveSessionTransitionState({
    currentSession,
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: {
      "agent-a": "reading",
      "agent-b": "writing",
    },
    layoutPreference: "individual",
  })

  assert.equal(sessionHasPromptWork(nextSession), true)
  assert.equal(transition.nextFocusedAgentId, "agent-b")
  assert.equal(transition.nextStreamingAgentId, "agent-b")
  assert.equal(transition.nextFocusedActivityLabel, "writing")
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-a": null,
    "agent-b": "writing",
  })
  assert.equal(transition.nextLayout, "split")
  assert.equal(transition.nextWorking, true)
  assert.equal(transition.previousAgentSignature, "agent-a,agent-b")
  assert.equal(transition.nextAgentSignature, "agent-a,agent-b")
})

test("sessionHasPromptWork and agentHasPromptWork honor prompt_states across agents", () => {
  const nextSession = session({
    focused_agent_id: "agent-a",
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-b": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-b",
          prompt: "review",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [agent("agent-a"), agent("agent-b", { is_processing: true, state: "Working" })],
  })

  assert.equal(sessionHasPromptWork(nextSession), true)
  assert.equal(agentHasPromptWork(nextSession, "agent-a"), false)
  assert.equal(agentHasPromptWork(nextSession, "agent-b"), true)
})

test("sessionHasPromptWork and agentHasPromptWork prefer kernel agent activity", () => {
  const nextSession = session({
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "settling",
        busy: true,
      },
      "agent-b": {
        status: "idle",
        prompt_status: "none",
        busy: false,
      },
    },
    agents: [agent("agent-a"), agent("agent-b")],
  })

  assert.equal(sessionHasPromptWork(nextSession), true)
  assert.equal(agentHasPromptWork(nextSession, "agent-a"), true)
  assert.equal(agentHasPromptWork(nextSession, "agent-b"), false)
})

test("shouldConfirmIdleTurnCompletion treats idle session snapshots as stale-turn completion", () => {
  const idleSession = session({
    agents: [agent("agent-a", { state: "Focused" }), agent("agent-b")],
  })

  assert.equal(sessionHasPromptWork(idleSession), false)
  assert.equal(sessionHasProcessingAgent(idleSession), false)
  assert.equal(shouldConfirmIdleTurnCompletion({
    nextSession: idleSession,
    currentWorking: true,
    currentSubmitting: false,
    currentBusyLatches: {},
    currentStreamingAgentId: "agent-a",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
  }), true)
})

test("shouldConfirmIdleTurnCompletion does not override active prompt or processing snapshots", () => {
  const activePromptSession = session({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "hello",
      status: "running",
    },
    agents: [agent("agent-a", { is_processing: false, state: "Focused" })],
  })
  const processingSession = session({
    agents: [agent("agent-a", { is_processing: true, state: "Working" })],
  })

  for (const nextSession of [activePromptSession, processingSession]) {
    assert.equal(shouldConfirmIdleTurnCompletion({
      nextSession,
      currentWorking: true,
      currentSubmitting: true,
      currentBusyLatches: { "agent-a": true },
      currentStreamingAgentId: "agent-a",
      currentProviderActivityLabel: "thinking",
      currentActiveStatusLabel: "thinking",
    }), false)
  }
})

test("agentPromptState tolerates daemon payloads that omit empty queued prompts", () => {
  const nextSession = session({
    focused_agent_id: "agent-a",
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-a",
          prompt: "hello",
          status: "running",
        },
      } as NonNullable<RuntimeSession["prompt_states"]>[string],
    },
  })

  assert.deepEqual(agentPromptState(nextSession, "agent-a"), {
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "hello",
      status: "running",
    },
    queued_prompts: [],
  })
})

test("deriveSessionTransitionState clears stale streaming state once prompt work ends", () => {
  const currentSession = session({
    focused_agent_id: "agent-a",
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "hello",
      status: "running",
    },
    agents: [agent("agent-a", { is_processing: true, state: "Working" }), agent("agent-b")],
  })
  const nextSession = session({
    focused_agent_id: "agent-a",
    agents: [agent("agent-a"), agent("agent-b")],
  })

  const transition = deriveSessionTransitionState({
    currentSession,
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-a",
    currentAgentActivityLabels: {
      "agent-a": "thinking",
      "agent-b": null,
    },
    layoutPreference: "split",
  })

  assert.equal(transition.nextStreamingAgentId, "agent-a")
  assert.equal(transition.nextFocusedActivityLabel, "thinking")
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-a": "thinking",
    "agent-b": null,
  })
  assert.equal(transition.nextWorking, true)
})

test("derivePromptLifecycleTransition detects when a cancelling prompt settles", () => {
  const transition = derivePromptLifecycleTransition(
    session({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "hello",
        status: "cancelling",
      },
    }),
    session(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-a"])
})

test("derivePromptLifecycleTransition ignores normal prompt replacement", () => {
  const transition = derivePromptLifecycleTransition(
    session({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "hello",
        status: "running",
      },
    }),
    session({
      active_prompt: {
        id: "prompt-2",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "next",
        status: "running",
      },
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-a"])
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
      providerId: "opencode",
      modelId: "openai/gpt-5.4",
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

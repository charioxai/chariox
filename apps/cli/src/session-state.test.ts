import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, CliOptions, RuntimeSession } from "./cli-types.js"
import {
  activeInteractionForAgent,
  activePromptIdForAgent,
  agentPromptState,
  agentHasPromptWork,
  deriveAttachedCliTransitionState,
  deriveDetachedCliTransitionState,
  buildDetachedSessionState,
  derivePromptLifecycleTransition,
  deriveSessionTransitionState,
  focusedAgentIdForSession,
  focusedProviderRunForAgent,
  projectedStreamingAgentIdForSession,
  promptWorkByAgent,
  sessionHasProjectedRuntimeState,
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

test("runtime session projections derive focus, interactions, provider run, and prompt work by agent", () => {
  const nextSession = session({
    focused_agent_id: null,
    agents: [agent("agent-a"), agent("agent-b")],
    active_interactions: [
      {
        id: "interaction-1",
        agent_id: "agent-b",
        kind: "permission",
        level: "info",
        title: "Approve?",
        message: "Approve?",
        choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }],
        requested_at_ms: 1,
      },
    ],
    prompt_states: {
      "agent-b": {
        active_prompt: null,
        queued_prompts: [{
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-b",
          prompt: "review",
          status: "queued",
        }],
      },
    },
  })

  const providerRun = providerRunFor("agent-a")

  assert.equal(focusedAgentIdForSession(nextSession), "agent-a")
  assert.equal(activeInteractionForAgent(nextSession, "agent-b")?.id, "interaction-1")
  assert.equal(activeInteractionForAgent(nextSession, null), null)
  assert.equal(focusedProviderRunForAgent(providerRun, "agent-a")?.id, "run-1")
  assert.equal(focusedProviderRunForAgent(providerRun, "agent-b"), null)
  assert.deepEqual(promptWorkByAgent(nextSession), {
    "agent-a": false,
    "agent-b": true,
  })
})

test("activePromptIdForAgent prefers projected active turn and per-agent prompt state", () => {
  assert.equal(activePromptIdForAgent(session({
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_turn: {
          prompt_id: "projected-prompt",
          status: "running",
          phase: "streaming",
        },
      },
    },
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "stale-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-a",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-a"), "projected-prompt")

  assert.equal(activePromptIdForAgent(session({
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-a",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-a"), "state-prompt")

  assert.equal(activePromptIdForAgent(session({
    agent_activity: {
      "agent-a": {
        status: "idle",
        prompt_status: "none",
        busy: false,
      },
    },
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "stale-idle-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-a",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-a"), null)
})

test("activePromptIdForAgent falls back to prompt state for session-wide sparse busy activity", () => {
  const nextSession = session({
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
      },
    },
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-a",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [agent("agent-a")],
  })

  assert.equal(activePromptIdForAgent(nextSession, null), "state-prompt")
  assert.equal(activePromptIdForAgent(nextSession, "agent-a"), "state-prompt")
})

test("activePromptIdForAgent suppresses prompt state for session-wide idle or missing activity", () => {
  const promptStates: NonNullable<RuntimeSession["prompt_states"]> = {
    "agent-a": {
      active_prompt: {
        id: "stale-a",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "stale",
        status: "running",
      },
      queued_prompts: [],
    },
    "agent-b": {
      active_prompt: {
        id: "stale-b",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-b",
        prompt: "stale",
        status: "running",
      },
      queued_prompts: [],
    },
  }

  const nextSession = session({
    agent_activity: {
      "agent-a": {
        status: "idle",
        prompt_status: "none",
        busy: false,
      },
    },
    prompt_states: promptStates,
    agents: [agent("agent-a"), agent("agent-b")],
  })

  assert.equal(activePromptIdForAgent(nextSession, null), null)
  assert.equal(activePromptIdForAgent(nextSession, "agent-a"), null)
  assert.equal(activePromptIdForAgent(nextSession, "agent-b"), null)
})

test("focusedAgentIdForSession does not fall back when focused id is not in the session", () => {
  assert.equal(focusedAgentIdForSession(session({
    focused_agent_id: "stale-agent",
    agents: [agent("agent-a"), agent("agent-b")],
  })), null)
  assert.equal(focusedAgentIdForSession(session({
    focused_agent_id: "stale-agent",
    agents: [],
  })), null)
})

test("focusedAgentIdForSession falls back only when no focused id is set", () => {
  assert.equal(focusedAgentIdForSession(session({
    focused_agent_id: null,
    agents: [agent("agent-a"), agent("agent-b")],
  })), "agent-a")
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

test("sessionHasProjectedRuntimeState treats prompt state as authoritative runtime projection", () => {
  assert.equal(sessionHasProjectedRuntimeState(session()), false)
  assert.equal(sessionHasProjectedRuntimeState(session({
    prompt_states: {},
  })), true)
  assert.equal(sessionHasProjectedRuntimeState(session({
    agent_activity: {},
  })), true)
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

test("kernel agent activity busy predicate includes status, prompt status, and active turn", () => {
  for (const activity of [
    { status: "working", prompt_status: "none", busy: false },
    { status: "idle", prompt_status: "settling", busy: false },
    {
      status: "idle",
      prompt_status: "none",
      busy: false,
      active_turn: {
        prompt_id: "prompt-1",
        status: "running",
        phase: "streaming",
      },
    },
  ] satisfies NonNullable<RuntimeSession["agent_activity"]>[string][]) {
    const nextSession = session({
      agent_activity: {
        "agent-a": activity,
      },
      agents: [agent("agent-a")],
    })

    assert.equal(sessionHasPromptWork(nextSession), true)
    assert.equal(sessionHasProcessingAgent(nextSession), true)
    assert.equal(agentHasPromptWork(nextSession, "agent-a"), true)
    assert.equal(projectedStreamingAgentIdForSession(nextSession), "agent-a")
  }
})

test("kernel agent activity suppresses stale legacy prompt activity", () => {
  const nextSession = session({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale top-level prompt",
      status: "running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-b",
      prompt: "stale queued prompt",
      status: "queued",
    }],
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "prompt-state-stale",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-a",
          prompt: "stale prompt state",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
    agents: [agent("agent-a"), agent("agent-b")],
  })

  assert.equal(sessionHasPromptWork(nextSession), false)
  assert.equal(agentHasPromptWork(nextSession, "agent-a"), false)
  assert.equal(agentHasPromptWork(nextSession, "agent-b"), false)
  assert.deepEqual(promptWorkByAgent(nextSession), {
    "agent-a": false,
    "agent-b": false,
  })
})

test("prompt_states suppress stale legacy prompt activity even when empty", () => {
  const nextSession = session({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale top-level prompt",
      status: "running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale queued prompt",
      status: "queued",
    }],
    prompt_states: {},
    agents: [agent("agent-a")],
  })

  assert.equal(sessionHasPromptWork(nextSession), false)
  assert.equal(agentHasPromptWork(nextSession, "agent-a"), false)
  assert.equal(activePromptIdForAgent(nextSession, "agent-a"), null)
  assert.equal(projectedStreamingAgentIdForSession(nextSession), null)
  assert.equal(agentPromptState(nextSession, "agent-a"), null)
})

test("explicit empty agent prompt state suppresses stale top-level prompts", () => {
  const nextSession = session({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale top-level prompt",
      status: "running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale queued prompt",
      status: "queued",
    }],
    prompt_states: {
      "agent-a": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agents: [agent("agent-a")],
  })

  assert.equal(sessionHasPromptWork(nextSession), false)
  assert.equal(agentHasPromptWork(nextSession, "agent-a"), false)
  assert.equal(activePromptIdForAgent(nextSession, "agent-a"), null)
  assert.equal(projectedStreamingAgentIdForSession(nextSession), null)
  assert.deepEqual(agentPromptState(nextSession, "agent-a"), {
    active_prompt: null,
    queued_prompts: [],
  })
})

test("projectedStreamingAgentIdForSession uses projected activity before legacy active prompts", () => {
  assert.equal(projectedStreamingAgentIdForSession(session({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale",
      status: "running",
    },
    agent_activity: {
      "agent-a": { status: "idle", prompt_status: "none", busy: false },
      "agent-b": { status: "working", prompt_status: "running", busy: true },
    },
    agents: [agent("agent-a"), agent("agent-b")],
  })), "agent-b")

  assert.equal(projectedStreamingAgentIdForSession(session({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale",
      status: "running",
    },
    agent_activity: {},
    agents: [agent("agent-a")],
  })), null)

  assert.equal(projectedStreamingAgentIdForSession(session({
    active_prompt: {
      id: "prompt-legacy",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "legacy",
      status: "running",
    },
    agents: [agent("agent-a")],
  })), "agent-a")
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

test("deriveSessionTransitionState ignores stale active prompt target when projected activity is idle", () => {
  const nextSession = session({
    focused_agent_id: "agent-a",
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-a",
      prompt: "stale",
      status: "running",
    },
    agent_activity: {},
    agents: [agent("agent-a")],
  })

  const transition = deriveSessionTransitionState({
    currentSession: session({ agents: [agent("agent-a")] }),
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: {
      "agent-a": "thinking",
    },
    layoutPreference: "individual",
  })

  assert.equal(transition.nextHasPromptWork, false)
  assert.equal(transition.nextStreamingAgentId, null)
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-a": null,
  })
})

test("deriveSessionTransitionState ignores stale processing state when projected activity is idle", () => {
  const nextSession = session({
    focused_agent_id: "agent-a",
    agent_activity: {
      "agent-a": {
        status: "idle",
        prompt_status: "none",
        busy: false,
      },
    },
    agents: [agent("agent-a", { is_processing: true, state: "Working" })],
  })

  const transition = deriveSessionTransitionState({
    currentSession: session({ agents: [agent("agent-a")] }),
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-a",
    currentAgentActivityLabels: {
      "agent-a": "thinking",
    },
    layoutPreference: "individual",
  })

  assert.equal(transition.nextHasPromptWork, false)
  assert.equal(transition.nextStreamingAgentId, null)
  assert.equal(transition.nextFocusedActivityLabel, null)
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-a": null,
  })
})

test("deriveSessionTransitionState ignores stale processing state when prompt state is idle", () => {
  const nextSession = session({
    focused_agent_id: "agent-a",
    prompt_states: {
      "agent-a": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agents: [agent("agent-a", { is_processing: true, state: "Working" })],
  })

  const transition = deriveSessionTransitionState({
    currentSession: session({ agents: [agent("agent-a")] }),
    nextSession,
    currentWorking: true,
    currentStreamingAgentId: "agent-a",
    currentAgentActivityLabels: {
      "agent-a": "thinking",
    },
    layoutPreference: "individual",
  })

  assert.equal(sessionHasProcessingAgent(nextSession), false)
  assert.equal(transition.nextHasPromptWork, false)
  assert.equal(transition.nextStreamingAgentId, null)
  assert.equal(transition.nextFocusedActivityLabel, null)
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-a": null,
  })
})

test("deriveSessionTransitionState resolves streaming from active prompt state before stale processing agents", () => {
  const nextSession = session({
    focused_agent_id: "agent-b",
    prompt_states: {
      "agent-a": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-b": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-b",
          prompt: "hello",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [
      agent("agent-a", { is_processing: true, state: "Working" }),
      agent("agent-b", { is_processing: false, state: "Idle" }),
    ],
  })

  const transition = deriveSessionTransitionState({
    currentSession: session({ agents: [agent("agent-a"), agent("agent-b")] }),
    nextSession,
    currentWorking: false,
    currentStreamingAgentId: null,
    currentAgentActivityLabels: {
      "agent-a": "thinking",
      "agent-b": null,
    },
    layoutPreference: "individual",
  })

  assert.equal(sessionHasProcessingAgent(nextSession), true)
  assert.equal(transition.nextHasPromptWork, true)
  assert.equal(transition.nextStreamingAgentId, "agent-b")
  assert.deepEqual(transition.nextAgentActivityLabels, {
    "agent-a": null,
    "agent-b": null,
  })
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

test("derivePromptLifecycleTransition treats projected idle activity as prompt settlement", () => {
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
    session({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "stale",
        status: "cancelling",
      },
      agent_activity: {},
    }),
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

test("derivePromptLifecycleTransition settles external prompts when they disappear", () => {
  const transition = derivePromptLifecycleTransition(
    session({
      agent_activity: {
        "agent-a": {
          status: "working",
          prompt_status: "running",
          busy: true,
          active_turn: {
            prompt_id: "prompt-1",
            status: "running",
            prompt_origin: "external",
            phase: "streaming",
          },
        },
      },
    }),
    session(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-a"])
})

test("derivePromptLifecycleTransition settles external prompts regardless of origin casing", () => {
  const transition = derivePromptLifecycleTransition(
    session({
      agent_activity: {
        "agent-a": {
          status: "working",
          prompt_status: "running",
          busy: true,
          active_turn: {
            prompt_id: "prompt-1",
            status: "running",
            prompt_origin: " External ",
            phase: "streaming",
          },
        },
      },
    }),
    session(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-a"])
})

test("derivePromptLifecycleTransition still settles external prompts with explicit cancellation", () => {
  const transition = derivePromptLifecycleTransition(
    session({
      agent_activity: {
        "agent-a": {
          status: "working",
          prompt_status: "cancelling",
          busy: true,
          active_turn: {
            prompt_id: "prompt-1",
            status: "cancelling",
            prompt_origin: "External",
            phase: "settling",
          },
        },
      },
    }),
    session(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
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

function providerRunFor(agentId: string) {
  return {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: agentId,
    adapter_key: "opencode",
    provider: "opencode",
    account_profile: "default",
    model: "model",
    variant: null,
    usage_tokens_total: null,
    state: "running",
  }
}

import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionActivePromptIdForAgent,
  sessionActivePromptLifecycleRecords,
  sessionAgentHasUnreadIdleOutput,
  sessionAgentIsBusy,
  sessionAgentRuntimeDisplayState,
  sessionAgentRuntimeState,
  sessionHasActivePrompt,
  sessionHasProcessingAgent,
  sessionHasPromptWork,
  sessionPromptLifecycleTransition,
  sessionPromptForAgent,
  sessionPromptWorkSummary,
  sessionShouldConfirmIdleTurnCompletion,
} from "./shell-agent-activity.js"
import { makeAgent, makeSession } from "./shell-executor.test-support.js"

test("sessionAgentIsBusy uses projected idle over stale legacy prompt state", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-1"), false)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({
    id: "agent-1",
    state: "Working",
    is_processing: true,
  })), "Idle")
  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Working",
    is_processing: true,
  })), "Idle")
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
})

test("sessionAgentIsBusy treats missing projected agent activity as idle", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 0,
    queued: 0,
    busyAgents: 0,
  })
})

test("sessionAgentRuntimeDisplayState maps unfocused unread idle output to done", () => {
  const session = makeSession({
    focused_agent_id: "agent-focused",
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
      "agent-focused": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
    },
  })

  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-1"), true)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Done")
  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-focused"), false)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({
    id: "agent-focused",
    state: "Idle",
    is_processing: false,
  })), "Idle")
})

test("sessionPromptWorkSummary counts projected active turns and prompt state queues", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "queued",
          status: "Queued",
        }],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-2",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 1,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary prefers projected prompt counts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: null,
        queued_prompts: [{
          id: "stale-queued",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "stale queued",
          status: "Queued",
        }],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_prompt_count: 1,
        queued_prompt_count: 2,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 2,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary ignores settled active turn statuses", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: malformedRuntimeValue("completed"),
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: malformedRuntimeValue(" Completed "),
          phase: malformedRuntimeValue("settled"),
        },
      },
      "agent-2": {
        status: "idle",
        prompt_status: malformedRuntimeValue("cancelled"),
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-2",
          prompt_origin: "external",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
      "agent-3": {
        status: "idle",
        prompt_status: "settling",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-3",
          provider_run_id: "run-3",
          prompt_origin: "external",
          status: malformedRuntimeValue(" settling "),
          phase: "settling",
        },
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary counts prompt state active prompt for sparse busy activity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-2",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "none",
        busy: true,
        unread_idle_output: false,
      },
      "agent-2": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionPromptWorkSummary treats prompt states as runtime authority", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-2",
          target_agent_id: "agent-2",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-3": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-3",
          source_attachment_id: "attach-3",
          target_agent_id: "agent-3",
          prompt: "queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-2", state: "Idle", is_processing: false }),
      makeAgent({ id: "agent-3", state: "Idle", is_processing: false }),
    ],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 1,
    busyAgents: 2,
  })
})

test("sessionShouldConfirmIdleTurnCompletion treats idle snapshots as stale-turn completion", () => {
  const idleSession = makeSession({
    agents: [makeAgent({ id: "agent-1", state: "Focused" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionHasPromptWork(idleSession), false)
  assert.equal(sessionHasProcessingAgent(idleSession), false)
  assert.equal(sessionShouldConfirmIdleTurnCompletion({
    nextSession: idleSession,
    currentWorking: true,
    currentSubmitting: false,
    currentBusyLatches: {},
    currentStreamingAgentId: "agent-1",
    currentProviderActivityLabel: "thinking",
    currentActiveStatusLabel: "thinking",
  }), true)
})

test("sessionShouldConfirmIdleTurnCompletion does not override active prompt or processing snapshots", () => {
  const activePromptSession = makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: "running",
    },
    agents: [makeAgent({ id: "agent-1", is_processing: false, state: "Focused" })],
  })
  const processingSession = makeSession({
    agents: [makeAgent({ id: "agent-1", is_processing: true, state: "Working" })],
  })

  for (const nextSession of [activePromptSession, processingSession]) {
    assert.equal(sessionShouldConfirmIdleTurnCompletion({
      nextSession,
      currentWorking: true,
      currentSubmitting: true,
      currentBusyLatches: { "agent-1": true },
      currentStreamingAgentId: "agent-1",
      currentProviderActivityLabel: "thinking",
      currentActiveStatusLabel: "thinking",
    }), false)
  }
})

test("sessionPromptWorkSummary ignores prompt states for agents outside the session", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost running",
          status: "Running",
        },
        queued_prompts: [{
          id: "queued-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    ],
  })

  assert.deepEqual(sessionPromptWorkSummary(session), {
    active: 1,
    queued: 0,
    busyAgents: 1,
  })
})

test("sessionAgentRuntimeState normalizes projected error status", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: malformedRuntimeValue(" Error "),
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Error")
})

test("sessionHasActivePrompt follows projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-2",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Working")
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-2")
})

test("session prompt helpers ignore settled projected active turn identity", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "external",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionPromptForAgent rejects legacy prompts that do not match projected active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-2",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-2"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionHasActivePrompt does not invent prompt identity from anonymous projected activity", () => {
  const session = makeSession({
    prompt_states: {},
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers use prompt state identity when projected activity is busy without active turn", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-1")
})

test("sessionHasActivePrompt follows projected active turn even when prompt state is absent", () => {
  const session = makeSession({
    prompt_states: {},
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "arroba",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionHasActivePrompt falls back to legacy fields when projection is unavailable", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), true)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-1"), true)
  assert.equal(sessionPromptForAgent(session, "agent-1")?.id, "prompt-1")
})

test("session prompt helpers prefer explicit empty prompt state over stale top-level active prompt", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers treat missing prompt state agents as idle", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
    prompt_states: {},
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers ignore prompt states for agents outside the session", () => {
  const session = makeSession({
    prompt_states: {
      "agent-ghost": {
        active_prompt: {
          id: "prompt-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost running",
          status: "Running",
        },
        queued_prompts: [{
          id: "queued-ghost",
          source_attachment_id: "attach-ghost",
          target_agent_id: "agent-ghost",
          prompt: "ghost queued",
          status: "Queued",
        }],
      },
    },
    agents: [
      makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    ],
  })

  assert.equal(sessionAgentIsBusy(session, "agent-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "prompt-ghost"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-ghost", "queued-ghost"), false)
  assert.equal(sessionPromptForAgent(session, "agent-ghost"), null)
})

test("session prompt helpers prefer explicit empty prompt state over stale top-level queued prompts", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    queued_prompts: [{
      id: "queued-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale queued",
      status: "Queued",
    }],
  })

  assert.equal(sessionAgentIsBusy(session, "agent-1"), false)
  assert.equal(sessionHasActivePrompt(session, "agent-1", "queued-stale"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("session prompt helpers ignore top-level prompts for other agents", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-other",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "other",
      status: "Running",
    },
    queued_prompts: [{
      id: "queued-other",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-2",
      prompt: "other queued",
      status: "Queued",
    }],
  })

  assert.equal(sessionHasActivePrompt(session, "agent-1", "prompt-other"), false)
  assert.equal(sessionPromptForAgent(session, "agent-1"), null)
})

test("sessionActivePromptLifecycleRecords uses projected active turns and deterministic order", () => {
  const session = makeSession({
    agents: [
      makeAgent({ id: "agent-b", state: "Working", is_processing: true }),
      makeAgent({ id: "agent-a", state: "Working", is_processing: true }),
    ],
    prompt_states: {
      "agent-a": {
        active_prompt: {
          id: "prompt-a-stale",
          source_attachment_id: "attach-a",
          target_agent_id: "agent-a",
          prompt: "stale",
          status: "Running",
          prompt_origin: "arroba",
        },
        queued_prompts: [],
      },
      "agent-b": {
        active_prompt: {
          id: "prompt-b-state",
          source_attachment_id: "attach-b",
          target_agent_id: "agent-b",
          prompt: "running",
          status: "Running",
          prompt_origin: "external",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-a": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-a-live",
          status: "running",
          prompt_origin: "external",
          phase: "streaming",
        },
      },
      "agent-b": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "prompt-a-live",
    status: "running",
    promptOrigin: "external",
    target_agent_id: "agent-a",
  }, {
    id: "prompt-b-state",
    source_attachment_id: "attach-b",
    target_agent_id: "agent-b",
    prompt: "running",
    status: "Running",
    prompt_origin: "external",
    promptOrigin: "external",
  }])
})

test("sessionActivePromptIdForAgent prefers projected active turn and per-agent prompt state", () => {
  assert.equal(sessionActivePromptIdForAgent(makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "projected-prompt",
          status: "running",
          phase: "streaming",
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), "projected-prompt")

  assert.equal(sessionActivePromptIdForAgent(makeSession({
    active_prompt: null,
    queued_prompts: [],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), "state-prompt")

  assert.equal(sessionActivePromptIdForAgent(makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-idle-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
  }), "agent-1"), null)
})

test("sessionActivePromptIdForAgent falls back to prompt state for sparse busy activity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "state-prompt",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "running",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, null), "state-prompt")
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), "state-prompt")
})

test("sessionActivePromptIdForAgent ignores legacy active prompt for sparse activity without prompt state", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "stale top-level prompt",
      status: "running",
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
})

test("sessionActivePromptIdForAgent suppresses prompt state for idle or missing projected activity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "stale-a",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
      "agent-2": {
        active_prompt: {
          id: "stale-b",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-2",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" }), makeAgent({ id: "agent-2" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, null), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptIdForAgent(session, "agent-2"), null)
})

test("sessionActivePromptIdForAgent ignores settled projected active turn identity", () => {
  const session = makeSession({
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-settled",
          status: malformedRuntimeValue("cancelled"),
          phase: malformedRuntimeValue("settled"),
        },
      },
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "running",
        },
        queued_prompts: [],
      },
    },
    agents: [makeAgent({ id: "agent-1" })],
  })

  assert.equal(sessionActivePromptIdForAgent(session, "agent-1"), null)
  assert.equal(sessionActivePromptIdForAgent(session, null), null)
})

test("sessionActivePromptLifecycleRecords treats projected idle as authoritative", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-stale",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "stale",
      status: "cancelling",
    },
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-stale",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "stale",
          status: "cancelling",
        },
        queued_prompts: [],
      },
    },
    agent_activity: {},
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [])
})

test("sessionActivePromptLifecycleRecords falls back to legacy active prompt without projections", () => {
  const session = makeSession({
    active_prompt: {
      id: "prompt-legacy",
      source_attachment_id: "attach-1",
      target_agent_id: "agent-1",
      prompt: "legacy",
      status: "Running",
      prompt_origin: " External ",
    },
  })

  assert.deepEqual(sessionActivePromptLifecycleRecords(session), [{
    id: "prompt-legacy",
    source_attachment_id: "attach-1",
    target_agent_id: "agent-1",
    prompt: "legacy",
    status: "Running",
    prompt_origin: " External ",
    promptOrigin: " External ",
  }])
})

test("sessionPromptLifecycleTransition detects when a cancelling prompt settles", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "cancelling",
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition treats projected idle activity as prompt settlement", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "cancelling",
      },
    }),
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "stale",
        status: "cancelling",
      },
      agent_activity: {},
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition ignores already-settled projected active turns", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-settled",
            status: malformedRuntimeValue("cancelled"),
            phase: malformedRuntimeValue("settled"),
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, false)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, [])
})

test("sessionPromptLifecycleTransition detects normal prompt replacement", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: "running",
      },
    }),
    makeSession({
      active_prompt: {
        id: "prompt-2",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "next",
        status: "running",
      },
    }),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition settles external prompts when they disappear", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-1",
            status: "running",
            prompt_origin: " External ",
            phase: "streaming",
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, false)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

test("sessionPromptLifecycleTransition settles cancelling external prompts", () => {
  const transition = sessionPromptLifecycleTransition(
    makeSession({
      agent_activity: {
        "agent-1": {
          status: "working",
          prompt_status: "cancelling",
          busy: true,
          unread_idle_output: false,
          active_turn: {
            prompt_id: "prompt-1",
            status: "cancelling",
            prompt_origin: "External",
            phase: "settling",
          },
        },
      },
    }),
    makeSession(),
  )

  assert.equal(transition.activePromptChanged, true)
  assert.equal(transition.cancelledPromptSettled, true)
  assert.deepEqual(transition.settledAgentIds, ["agent-1"])
})

function malformedRuntimeValue<T>(value: string): T {
  return value as unknown as T
}

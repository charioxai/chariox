import assert from "node:assert/strict"
import test from "node:test"

import {
  agentRuntimeStateFromProjection,
  sessionAgentHasUnreadIdleOutput,
  sessionAgentPaneStatusBadge,
  sessionAgentPaneStatusBadgeForSession,
  sessionAgentRuntimeDisplayStateByAgent,
  sessionAgentRuntimeDisplayState,
  sessionAgentRuntimeDisplayStates,
  sessionAgentRuntimeState,
  sessionFocusedStatusBadge,
  sessionStatusLabel,
  sessionStatusMode,
} from "./session-runtime-status.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

test("session status mode follows disconnected and active turn precedence", () => {
  assert.equal(sessionStatusMode({
    daemonDisconnected: true,
    working: true,
    hasActiveTurnWork: true,
    submitting: true,
    queueDepth: 1,
  }), "disconnected")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActiveTurnWork: true,
    submitting: false,
    queueDepth: 0,
  }), "working")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActiveTurnWork: false,
    submitting: false,
    queueDepth: 1,
  }), "idle")
  assert.equal(sessionStatusMode({
    daemonDisconnected: false,
    working: false,
    hasActiveTurnWork: false,
    submitting: false,
    queueDepth: 0,
  }), "idle")
})

test("session status labels normalize active provider activity", () => {
  assert.equal(sessionStatusLabel("idle", "grepping"), "IDLE")
  assert.equal(sessionStatusLabel("disconnected", "grepping"), "DISCONNECTED")
  assert.equal(sessionStatusLabel("working", null), "THINKING")
  assert.equal(sessionStatusLabel("working", "grepping"), "GREPPING")
})

test("session focused status badge projects single and multi-agent states", () => {
  assert.deepEqual(sessionFocusedStatusBadge({
    attached: false,
    daemonDisconnected: false,
    activeStatusLabel: null,
    focusedBusy: false,
  }), {
    label: "",
    tone: "idle",
    parts: [],
  })
  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: true,
    activeStatusLabel: null,
    focusedBusy: false,
  }), {
    label: "DISCONNECTED",
    tone: "disconnected",
    parts: [{ label: "DISCONNECTED", tone: "disconnected" }],
  })
  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: "grepping",
    focusedBusy: true,
  }), {
    label: "GREPPING",
    tone: "working",
    parts: [{ label: "GREPPING", tone: "working" }],
  })
  assert.deepEqual(sessionFocusedStatusBadge({
    attached: true,
    daemonDisconnected: false,
    activeStatusLabel: null,
    focusedBusy: false,
    agents: [{ id: "agent-1", busy: false }, { id: "agent-2", busy: true }],
  }), {
    label: "1 IDLE 1 WORKING",
    tone: "working",
    parts: [
      { label: "1 IDLE", tone: "idle" },
      { label: "1 WORKING", tone: "working" },
    ],
  })
})

test("session agent pane status badge projects activity and prompt work", () => {
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: null,
    activeLabel: null,
  }), { label: "", tone: "idle" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: { state: "Error" },
    activeLabel: null,
  }), { label: "ERROR", tone: "error" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: { state: "Idle" },
    activeLabel: "editing",
  }), { label: "EDITING", tone: "working" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: { state: "Idle" },
    activeLabel: null,
    hasPromptWork: true,
  }), { label: "THINKING", tone: "working" })
  assert.deepEqual(sessionAgentPaneStatusBadge({
    agent: { state: "Working", is_processing: true },
    activeLabel: null,
    useLegacyAgentProcessingState: false,
  }), { label: "IDLE", tone: "idle" })
})

test("session agent pane status badge for session uses authoritative projection", () => {
  const session = makeSession({
    agents: [makeAgent({
      id: "agent-1",
      state: "Working",
      is_processing: true,
    })],
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.deepEqual(sessionAgentPaneStatusBadgeForSession({
    session,
    agent: session.agents[0],
    activeLabel: null,
  }), { label: "IDLE", tone: "idle" })

  assert.deepEqual(sessionAgentPaneStatusBadgeForSession({
    session: makeSession({
      agents: [makeAgent({ id: "agent-1", state: "Idle", is_processing: false })],
      prompt_states: {
        "agent-1": {
          active_prompt: {
            id: "prompt-1",
            source_attachment_id: "attach-1",
            target_agent_id: "agent-1",
            prompt: "run",
            status: "Running",
          },
        } as never,
      },
    }),
    agent: makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    activeLabel: null,
  }), { label: "THINKING", tone: "working" })

  assert.deepEqual(sessionAgentPaneStatusBadgeForSession({
    session: makeSession({
      agents: [makeAgent({ id: "agent-1", state: "Idle", is_processing: false })],
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
      },
    }),
    agent: makeAgent({ id: "agent-1", state: "Idle", is_processing: false }),
    activeLabel: null,
  }), { label: "IDLE", tone: "idle" })
})

test("session runtime state prefers projected activity over stale legacy state", () => {
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

  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Working",
    is_processing: true,
  })), "Idle")
  assert.equal(agentRuntimeStateFromProjection(makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  }), {
    agentActivity: {
      "agent-1": {
        status: "error",
        prompt_status: "none",
        busy: false,
      },
    },
  }), "Error")
})

test("session runtime state uses active prompt state without counting queues as working", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "run",
          status: "Running",
        },
      } as never,
    },
    agents: [makeAgent({ id: "agent-1", state: "Idle", is_processing: false })],
  })

  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Working")
  assert.equal(agentRuntimeStateFromProjection(makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  }), {
    promptStates: {
      "agent-1": {
        active_prompt: { id: "prompt-1" },
      },
    },
  }), "Working")

  const queuedOnlySession = makeSession({
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
    },
    agents: [makeAgent({ id: "agent-1", state: "Idle", is_processing: false })],
  })

  assert.equal(sessionAgentRuntimeState(queuedOnlySession, makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  })), "Idle")
  assert.equal(agentRuntimeStateFromProjection(makeAgent({
    id: "agent-1",
    state: "Idle",
    is_processing: false,
  }), {
    promptStates: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{ id: "queued-1" }],
      },
    },
  }), "Idle")
})

test("session runtime state ignores activity outside session agents", () => {
  const session = makeSession({
    agents: [makeAgent({
      id: "agent-1",
      state: "Working",
      is_processing: true,
    })],
    agent_activity: {
      "agent-ghost": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionAgentRuntimeState(session, makeAgent({
    id: "agent-1",
    state: "Working",
    is_processing: true,
  })), "Idle")
})

test("session runtime display marks unfocused unread idle output as done", () => {
  const session = makeSession({
    focused_agent_id: "agent-focused",
    agents: [makeAgent({ id: "agent-focused" }), makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-focused": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
    },
  })

  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-1"), true)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({ id: "agent-1" })), "Done")
  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-focused"), false)
  assert.equal(sessionAgentRuntimeDisplayState(session, makeAgent({ id: "agent-focused" })), "Idle")
  assert.deepEqual(sessionAgentRuntimeDisplayStates(session), [
    { id: "agent-focused", state: "Idle" },
    { id: "agent-1", state: "Done" },
  ])
  assert.deepEqual(sessionAgentRuntimeDisplayStateByAgent(session), {
    "agent-focused": "Idle",
    "agent-1": "Done",
  })
})

test("session runtime display ignores unread activity outside session agents", () => {
  const session = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    agent_activity: {
      "agent-ghost": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
    },
  })

  assert.equal(sessionAgentHasUnreadIdleOutput(session, "agent-ghost"), false)
})

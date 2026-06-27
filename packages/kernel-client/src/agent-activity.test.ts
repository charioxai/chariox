import assert from "node:assert/strict"
import test from "node:test"

import {
  agentRuntimeActiveTurnIsBusy,
  agentRuntimeActivityIsBusy,
  agentRuntimePromptStatusIsActive,
  agentRuntimePromptStatusIsActivePrompt,
  normalizeAgentRuntimeActivityProjectionStatus,
  normalizeAgentRuntimeActivityStatus,
  normalizeAgentRuntimePromptProjectionStatus,
  normalizeAgentRuntimePromptStatus,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"

test("agent activity busy helper follows kernel projected activity semantics", () => {
  assert.equal(agentRuntimeActivityIsBusy(null), false)
  assert.equal(agentRuntimeActivityIsBusy(undefined), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "working",
    prompt_status: "none",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: " Working ",
    prompt_status: "none",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "settling",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: " Cancelling ",
    busy: false,
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "completed",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "cancelled",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "unknown",
    busy: false,
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1" },
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1", status: "running" },
  }), true)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1", status: "completed" },
  }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_turn: { prompt_id: "prompt-1", status: "cancelled" },
  }), false)
  assert.equal(agentRuntimeActiveTurnIsBusy({ prompt_id: "prompt-1" }), true)
  assert.equal(agentRuntimeActiveTurnIsBusy({ prompt_id: "prompt-1", status: "completed" }), false)
  assert.equal(agentRuntimeActivityIsBusy({
    status: "idle",
    prompt_status: "none",
    busy: true,
  }), true)
})

test("agent activity helpers normalize status vocabulary", () => {
  assert.equal(normalizeAgentRuntimeActivityStatus(" Working "), "working")
  assert.equal(normalizeAgentRuntimeActivityStatus(""), null)
  assert.equal(normalizeAgentRuntimePromptStatus(" Cancelling "), "cancelling")
  assert.equal(normalizeAgentRuntimePromptStatus(""), null)
  assert.equal(normalizeAgentRuntimeActivityProjectionStatus(" focused "), "idle")
  assert.equal(normalizeAgentRuntimePromptProjectionStatus(" completed "), "none")
  assert.equal(normalizeAgentRuntimePromptProjectionStatus(" cancelled "), "none")
  assert.equal(agentRuntimePromptStatusIsActive("queued"), true)
  assert.equal(agentRuntimePromptStatusIsActive("completed"), false)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("queued"), false)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("running"), true)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("cancelled"), false)
})

test("agent activity projection preserves kernel counts as activity source", () => {
  assert.deepEqual(projectAgentRuntimeActivity({
    status: "idle",
    prompt_status: "none",
    busy: false,
    active_prompt_count: 0,
    queued_prompt_count: 2,
  }), {
    status: "idle",
    promptStatus: "none",
    busy: true,
    activeTurn: null,
    activePromptCount: 0,
    queuedPromptCount: 2,
    error: false,
  })
})

test("agent activity projection unwraps nested activity and normalizes settled statuses", () => {
  const activeTurn = { prompt_id: "prompt-1", status: "completed" }
  assert.deepEqual(projectAgentRuntimeActivity({
    activity: {
      status: "focused",
      prompt_status: "cancelled",
      busy: false,
      active_turn: activeTurn,
    },
  }), {
    status: "idle",
    promptStatus: "none",
    busy: false,
    activeTurn,
    activePromptCount: 0,
    queuedPromptCount: 0,
    error: false,
  })
})

test("agent activity projection preserves previous error only when kernel omits error state", () => {
  assert.equal(projectAgentRuntimeActivity({}, { previousError: true }).error, true)
  assert.equal(projectAgentRuntimeActivity({ status: "error" }, { previousError: false }).error, true)
  assert.equal(projectAgentRuntimeActivity({ status: "idle" }, { previousError: true }).error, false)
  assert.equal(projectAgentRuntimeActivity({ error: false }, { previousError: true }).error, false)
})

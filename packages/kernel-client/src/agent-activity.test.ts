import assert from "node:assert/strict"
import test from "node:test"

import {
  agentRuntimeActivityIsBusy,
  agentRuntimePromptStatusIsActive,
  agentRuntimePromptStatusIsActivePrompt,
  normalizeAgentRuntimeActivityStatus,
  normalizeAgentRuntimePromptStatus,
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
  assert.equal(agentRuntimePromptStatusIsActive("queued"), true)
  assert.equal(agentRuntimePromptStatusIsActive("completed"), false)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("queued"), false)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("running"), true)
  assert.equal(agentRuntimePromptStatusIsActivePrompt("cancelled"), false)
})

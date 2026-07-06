import assert from "node:assert/strict"
import test from "node:test"

import { createAuthoritativeIdleController } from "./authoritative-idle-controller.js"
import type { AgentInstance, RuntimeSession } from "./cli-types.js"

test("authoritative idle controller clears local busy state when the session is idle", () => {
  const harness = idleHarness({ statusLine: "Cancellation requested." })

  const cleared = harness.controller.clear(session())

  assert.equal(cleared, true)
  assert.deepEqual(harness.calls, [
    "batch:start",
    "turn:reset",
    "tools:clear",
    "activity:{}",
    "streaming:null",
    "submitting:false",
    "submitting-agent:clear",
    "stop:reset",
    "busy:{}",
    "provider:null",
    "active:null",
    "working:false",
    "status:",
    "batch:end",
    "render",
  ])
})

test("authoritative idle controller leaves local state alone while active turn work remains", () => {
  const harness = idleHarness()

  const cleared = harness.controller.clear(session({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "run",
      status: "Active",
    },
  }))

  assert.equal(cleared, false)
  assert.deepEqual(harness.calls, [])
})

test("authoritative idle controller clears local busy state when only queued prompts remain", () => {
  const harness = idleHarness({ statusLine: "Cancellation requested." })

  const cleared = harness.controller.clear(session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [{
          id: "queued-1",
          source_attachment_id: "attachment-1",
          target_agent_id: "agent-1",
          prompt: "next",
          status: "Queued",
        }],
      },
    },
  }))

  assert.equal(cleared, true)
  assert.deepEqual(harness.calls, [
    "batch:start",
    "turn:reset",
    "tools:clear",
    "activity:{}",
    "streaming:null",
    "submitting:false",
    "submitting-agent:clear",
    "stop:reset",
    "busy:{}",
    "provider:null",
    "active:null",
    "working:false",
    "status:",
    "batch:end",
    "render",
  ])
})

test("authoritative idle controller leaves local state alone while an agent is processing", () => {
  const harness = idleHarness()

  const cleared = harness.controller.clear(session({
    agents: [agent("agent-1", { state: "Working", is_processing: true })],
  }))

  assert.equal(cleared, false)
  assert.deepEqual(harness.calls, [])
})

function idleHarness(options: { statusLine?: string } = {}) {
  const harness = {
    statusLine: options.statusLine ?? "",
    calls: [] as string[],
    controller: null as ReturnType<typeof createAuthoritativeIdleController> | null,
  }
  harness.controller = createAuthoritativeIdleController({
    batchUpdate: (callback) => {
      harness.calls.push("batch:start")
      callback()
      harness.calls.push("batch:end")
    },
    resetTurnCompletion: () => harness.calls.push("turn:reset"),
    clearActiveToolLabels: () => harness.calls.push("tools:clear"),
    setAgentActivityLabels: (labels) => harness.calls.push(`activity:${JSON.stringify(labels)}`),
    setStreamingAgentId: (agentId) => harness.calls.push(`streaming:${agentId ?? "null"}`),
    setSubmitting: (submitting) => harness.calls.push(`submitting:${String(submitting)}`),
    clearSubmittingAgentId: () => harness.calls.push("submitting-agent:clear"),
    resetPromptStop: () => harness.calls.push("stop:reset"),
    setAgentBusyLatches: (latches) => harness.calls.push(`busy:${JSON.stringify(latches)}`),
    setProviderActivityLabel: (label) => harness.calls.push(`provider:${label ?? "null"}`),
    setActiveStatusLabel: (label) => harness.calls.push(`active:${label ?? "null"}`),
    setWorking: (working) => harness.calls.push(`working:${String(working)}`),
    getStatusLine: () => harness.statusLine,
    setStatusLine: (statusLine) => {
      harness.statusLine = statusLine
      harness.calls.push(`status:${statusLine}`)
    },
    renderSessionChromeBoundary: () => harness.calls.push("render"),
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAuthoritativeIdleController>
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 8,
    agents: [agent("agent-1")],
    config_state: { version: 1, values: {} },
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
    model: "gpt-5.4",
    worktree_id: null,
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

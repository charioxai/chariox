import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance } from "./cli-types.js"
import { resolveTerminalRecordAgentId } from "./terminal-record-agent-resolver.js"

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "s1",
    alias: null,
    provider: "opencode",
    model: "default",
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
  } as AgentInstance
}

test("resolveTerminalRecordAgentId prefers explicit record agent ids", () => {
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: "record-agent" },
    streamingAgentId: "streaming-agent",
    activePromptAgentId: "prompt-agent",
    agents: [agent("working-agent", { state: "Working" })],
    focusedAgentId: "focused-agent",
  }), "record-agent")
})

test("resolveTerminalRecordAgentId falls back through streaming, prompt, processing, and focus", () => {
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: null },
    streamingAgentId: "streaming-agent",
    activePromptAgentId: "prompt-agent",
    agents: [agent("working-agent", { state: "Working" })],
    focusedAgentId: "focused-agent",
  }), "streaming-agent")
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: null },
    streamingAgentId: null,
    activePromptAgentId: "prompt-agent",
    agents: [agent("working-agent", { state: "Working" })],
    focusedAgentId: "focused-agent",
  }), "prompt-agent")
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: null },
    streamingAgentId: null,
    activePromptAgentId: null,
    agents: [agent("working-agent", { is_processing: true })],
    focusedAgentId: "focused-agent",
  }), "working-agent")
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: null },
    streamingAgentId: null,
    activePromptAgentId: null,
    agents: [agent("idle-agent")],
    focusedAgentId: "focused-agent",
  }), "focused-agent")
})

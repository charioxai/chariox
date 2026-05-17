import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance } from "./cli-types.js"
import { formatAgentLabel } from "./agent-label.js"

test("formatAgentLabel includes alias when present", () => {
  assert.equal(formatAgentLabel(agent({ alias: "Builder" })), "agent-a (Builder)")
})

test("formatAgentLabel falls back to an empty label without an agent", () => {
  assert.equal(formatAgentLabel(null), "")
})

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: "gpt-5",
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

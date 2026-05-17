import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { resolveSessionAgentReference } from "./session-agent-resolver.js"

test("resolveSessionAgentReference returns the focused agent without an explicit reference", () => {
  const session = sessionWithAgents([
    agent({ id: "agent-1", agent_ref: "a1" }),
    agent({ id: "agent-2", agent_ref: "a2" }),
  ])

  assert.equal(resolveSessionAgentReference(session, "agent-2").agent?.id, "agent-2")
})

test("resolveSessionAgentReference resolves id, agent ref, and alias", () => {
  const session = sessionWithAgents([
    agent({ id: "agent-1", agent_ref: "a1", alias: "build" }),
  ])

  assert.equal(resolveSessionAgentReference(session, null, "agent-1").agent?.id, "agent-1")
  assert.equal(resolveSessionAgentReference(session, null, "a1").agent?.id, "agent-1")
  assert.equal(resolveSessionAgentReference(session, null, "build").agent?.id, "agent-1")
})

test("resolveSessionAgentReference reports ambiguous and missing references", () => {
  const session = sessionWithAgents([
    agent({ id: "agent-1", agent_ref: "a1", alias: "shared" }),
    agent({ id: "agent-2", agent_ref: "a2", alias: "shared" }),
  ])

  assert.deepEqual(resolveSessionAgentReference(session, null, "shared"), {
    agent: null,
    error: "multiple agents match 'shared'",
  })
  assert.deepEqual(resolveSessionAgentReference(session, null, "unknown"), {
    agent: null,
    error: "agent 'unknown' not found",
  })
})

test("resolveSessionAgentReference reports missing focus when the session has no agents", () => {
  assert.deepEqual(resolveSessionAgentReference(sessionWithAgents([]), null), {
    agent: null,
    error: "no focused agent available",
  })
})

function sessionWithAgents(agents: AgentInstance[]): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: agents[0]?.id ?? null,
    max_agents: agents.length,
    agents,
    config_state: { version: 1, values: {} },
  }
}

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

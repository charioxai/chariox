import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeInteraction,
  RuntimeSession,
} from "./cli-types.js"
import {
  createInteractionProjectionController,
} from "./interaction-projection-controller.js"

test("interaction projection resolves interactions for arbitrary agents", () => {
  let currentSession = session("agent-a", [interaction("interaction-a", "agent-a")])
  const controller = createInteractionProjectionController({
    getSession: () => currentSession,
    getFocusedAgentId: () => "agent-a",
  })

  assert.equal(controller.activeInteractionForAgent("agent-a")?.id, "interaction-a")
  assert.equal(controller.activeInteractionForAgent("agent-b"), null)

  currentSession = session("agent-b", [interaction("interaction-b", "agent-b")])

  assert.equal(controller.activeInteractionForAgent("agent-b")?.id, "interaction-b")
})

test("interaction projection follows the focused agent", () => {
  let focusedAgentId: string | null = "agent-a"
  const controller = createInteractionProjectionController({
    getSession: () => session("agent-a", [
      interaction("interaction-a", "agent-a"),
      interaction("interaction-b", "agent-b"),
    ]),
    getFocusedAgentId: () => focusedAgentId,
  })

  assert.equal(controller.focusedAgentInteraction()?.id, "interaction-a")

  focusedAgentId = "agent-b"

  assert.equal(controller.focusedAgentInteraction()?.id, "interaction-b")

  focusedAgentId = null

  assert.equal(controller.focusedAgentInteraction(), null)
})

function session(
  focusedAgentId: string,
  activeInteractions: RuntimeInteraction[],
): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Created",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: focusedAgentId,
    max_agents: 2,
    agents: [agent("agent-a"), agent("agent-b")],
    active_interactions: activeInteractions,
    config_state: { values: {} } as RuntimeSession["config_state"],
  }
}

function agent(id: string): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: null,
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
}

function interaction(id: string, agentId: string): RuntimeInteraction {
  return {
    id,
    agent_id: agentId,
    kind: "permission",
    level: "warning",
    message: "Allow action?",
    choices: [],
    requested_at_ms: 1,
  }
}

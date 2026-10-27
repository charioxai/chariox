import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "./cli-types.js"
import { createResponsePaneProjectionController } from "./response-pane-projection-controller.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"

test("response pane projection selects split panes around focused agents", () => {
  const session = runtimeSession({
    focused_agent_id: "agent-c",
    agents: [
      agent("agent-a"),
      agent("agent-b"),
      agent("agent-c"),
      agent("agent-d"),
    ],
  })
  const controller = createResponsePaneProjectionController({
    isAttached: () => true,
    getSession: () => session,
    getFocusedAgentId: () => "agent-c",
    getWorkspaceScreenMode: () => "agents",
    getResponseLayout: () => "split",
    getMaxAgentsPerScreen: () => 2,
    workflowScreenActive: () => false,
  })

  assert.equal(controller.multiAgentMode(), true)
  assert.equal(controller.splitAgentResponseMode(), true)
  assert.deepEqual(
    controller.responsePaneSelection().visibleAgents.map((item) => item.id),
    ["agent-c", "agent-d"],
  )
  assert.equal(controller.responsePrimaryAgent()?.id, "agent-c")
  assert.equal(controller.visibleTranscriptAgentId(), "agent-c")
  assert.deepEqual(controller.responsePaneRows(), [[0, 1]])
  assert.equal(controller.primaryTranscriptSurfaceTone(), "focused")
  assert.equal(controller.auxiliaryTranscriptSurfaceTone("agent-d"), "faded")
})

test("response pane projection hides agent panes while workflow screen is active", () => {
  const controller = createResponsePaneProjectionController({
    isAttached: () => true,
    getSession: () => runtimeSession({
      focused_agent_id: "agent-a",
      agents: [agent("agent-a"), agent("agent-b")],
    }),
    getFocusedAgentId: () => "agent-a",
    getWorkspaceScreenMode: () => "workflow" as WorkspaceScreenMode,
    getResponseLayout: () => "split",
    getMaxAgentsPerScreen: () => 3,
    workflowScreenActive: () => true,
  })

  assert.equal(controller.workflowScreenShowing(), true)
  assert.equal(controller.responsePrimaryAgent(), null)
  assert.deepEqual(controller.responseVisibleAgents(), [])
  assert.equal(controller.visibleTranscriptAgentId(), null)
})

function runtimeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
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

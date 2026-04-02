import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, WorkflowDefinition } from "./cli-types.js"
import {
  DEFAULT_WORKFLOW_ZOOM_INDEX,
  buildWorkflowGraphLayout,
  cycleWorkflowNodeId,
  derivePointerAnchoredViewport,
  deriveWorkflowZoomIndex,
  resolveSelectedWorkflow,
  resolveSelectedWorkflowNodeId,
} from "./workflow-graph.js"

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "openai/gpt-5",
    worktree_id: "worktree-1",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}

function workflow(): WorkflowDefinition {
  return {
    id: "flow-1",
    alias: "review",
    nodes: [
      { id: "node-a", agent_id: "agent-a" },
      { id: "node-b", agent_id: "agent-b" },
      { id: "node-c", agent_id: "agent-c" },
    ],
    edges: [
      { id: "edge-a", from_node_id: "node-a", to_node_id: "node-b" },
      { id: "edge-b", from_node_id: "node-b", to_node_id: "node-c" },
    ],
    endpoints: [
      { id: "endpoint-a", alias: "start", entry_node_id: "node-a" },
    ],
  }
}

test("resolveSelectedWorkflow and node selection fall back safely", () => {
  const selectedWorkflow = resolveSelectedWorkflow([workflow()], "missing")
  assert.equal(selectedWorkflow?.id, "flow-1")
  assert.equal(resolveSelectedWorkflow([], "flow-1"), null)
  assert.equal(resolveSelectedWorkflowNodeId(selectedWorkflow ?? null, "missing"), "node-a")
})

test("cycleWorkflowNodeId advances around the workflow nodes", () => {
  assert.equal(cycleWorkflowNodeId(workflow(), null), "node-a")
  assert.equal(cycleWorkflowNodeId(workflow(), "node-a"), "node-b")
  assert.equal(cycleWorkflowNodeId(workflow(), "node-c"), "node-a")
})

test("buildWorkflowGraphLayout arranges the graph north-south and marks missing agents", () => {
  const layout = buildWorkflowGraphLayout({
    workflow: workflow(),
    agents: [
      agent("agent-a"),
      agent("agent-b", { alias: "reviewer" }),
    ],
    selectedNodeId: "node-b",
    zoomIndex: DEFAULT_WORKFLOW_ZOOM_INDEX,
  })

  const nodeA = layout.nodes.find((node) => node.id === "node-a")
  const nodeB = layout.nodes.find((node) => node.id === "node-b")
  const nodeC = layout.nodes.find((node) => node.id === "node-c")

  assert.ok(nodeA)
  assert.ok(nodeB)
  assert.ok(nodeC)
  assert.equal(nodeA!.y < nodeB!.y, true)
  assert.equal(nodeB!.y < nodeC!.y, true)
  assert.equal(nodeB!.selected, true)
  assert.equal(nodeC!.missing, true)
  assert.equal(layout.endpoints[0]?.entryNodeId, "node-a")
})

test("derivePointerAnchoredViewport preserves the pointer-relative anchor during zoom", () => {
  const viewport = derivePointerAnchoredViewport({
    viewportWidth: 80,
    viewportHeight: 24,
    pointerX: 20,
    pointerY: 8,
    scrollLeft: 30,
    scrollTop: 10,
    previousContentWidth: 200,
    previousContentHeight: 100,
    nextContentWidth: 300,
    nextContentHeight: 150,
  })

  assert.deepEqual(viewport, { x: 55, y: 19 })
  assert.equal(deriveWorkflowZoomIndex(DEFAULT_WORKFLOW_ZOOM_INDEX, "in") > DEFAULT_WORKFLOW_ZOOM_INDEX, true)
})

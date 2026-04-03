import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, WorkflowDefinition } from "./cli-types.js"
import {
  buildWorkflowGraphLayout,
  cycleWorkflowNodeId,
  routeWorkflowEdge,
  resolveSelectedWorkflow,
  resolveSelectedWorkflowNodeId,
  type WorkflowGraphNodeLayout,
} from "./workflow-graph/index.js"

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "openai/gpt-5",
    effort: null,
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

function layoutNode(id: string, x: number, y: number, width = 30, height = 8): WorkflowGraphNodeLayout {
  return {
    id,
    agentId: id,
    alias: null,
    provider: "opencode",
    model: "openai/gpt-5.4",
    effort: "high",
    missing: false,
    selected: false,
    x,
    y,
    width,
    height,
    lines: [id, "provider opencode", "model openai/gpt-5.4", "effort high"],
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
  assert.equal(nodeB!.lines[0], "agent-b (reviewer)")
  assert.equal(nodeB!.lines[1], "provider opencode")
  assert.equal(nodeB!.lines[2], "model openai/gpt-5")
  assert.equal(layout.endpoints[0]?.entryNodeId, "node-a")
})

test("buildWorkflowGraphLayout applies live selection metadata to the active agent node", () => {
  const layout = buildWorkflowGraphLayout({
    workflow: workflow(),
    agents: [
      agent("agent-a"),
      agent("agent-b", { provider: "opencode", model: "openai/gpt-5.4", effort: "high" }),
    ],
    selectedNodeId: "node-b",
  })
  const nodeB = layout.nodes.find((node) => node.id === "node-b")
  assert.ok(nodeB)
  assert.equal(nodeB!.lines[1], "provider opencode")
  assert.equal(nodeB!.lines[2], "model openai/gpt-5.4")
  assert.equal(nodeB!.lines[3], "effort high")
})

test("buildWorkflowGraphLayout uses per-agent effort when present", () => {
  const layout = buildWorkflowGraphLayout({
    workflow: workflow(),
    agents: [
      agent("agent-a"),
      agent("agent-b", { provider: "opencode", model: "openai/gpt-5.4", effort: "high" }),
    ],
    selectedNodeId: "node-b",
  })
  const nodeB = layout.nodes.find((node) => node.id === "node-b")
  assert.ok(nodeB)
  assert.equal(nodeB!.lines[1], "provider opencode")
  assert.equal(nodeB!.lines[2], "model openai/gpt-5.4")
  assert.equal(nodeB!.lines[3], "effort high")
})

test("routeWorkflowEdge connects nearest border centers across orientations", () => {
  const topNode = layoutNode("top", 10, 10)
  const bottomNode = layoutNode("bottom", 10, 30)
  const leftNode = layoutNode("left", 10, 10)
  const rightNode = layoutNode("right", 60, 10)

  const downward = routeWorkflowEdge(topNode, bottomNode)
  assert.deepEqual(downward[0], { x: 25, y: 17 })
  assert.deepEqual(downward[downward.length - 1], { x: 25, y: 30 })

  const upward = routeWorkflowEdge(bottomNode, topNode)
  assert.deepEqual(upward[0], { x: 25, y: 30 })
  assert.deepEqual(upward[upward.length - 1], { x: 25, y: 17 })

  const horizontal = routeWorkflowEdge(leftNode, rightNode)
  assert.deepEqual(horizontal, [
    { x: 39, y: 14 },
    { x: 60, y: 14 },
  ])
})

import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { buildWorkflowGraphDrillCases, validateWorkflowGraphLayout } from "./workflow-graph-drills.js"
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
    runStatus: null,
    missing: false,
    selected: false,
    x,
    y,
    width,
    height,
    lines: [id, "provider opencode", "model openai/gpt-5.4", "effort high", "status idle"],
  }
}

function workflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "flow-1",
    endpoint_id: "endpoint-a",
    entry_node_id: "node-a",
    status: "Running",
    invocation_prompt: "review this diff",
    active_node_run_id: "node-run-a",
    node_runs: [
      {
        id: "node-run-a",
        node_id: "node-a",
        agent_id: "agent-a",
        status: "Completed",
        summary: "done",
        created_at_ms: 1,
        started_at_ms: 2,
        completed_at_ms: 3,
      },
      {
        id: "node-run-b",
        node_id: "node-b",
        agent_id: "agent-b",
        status: "Running",
        summary: null,
        created_at_ms: 4,
        started_at_ms: 5,
        completed_at_ms: null,
      },
    ],
    messages: [],
    created_at_ms: 10,
    started_at_ms: 11,
    completed_at_ms: null,
    ...overrides,
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
    workflowRuns: [],
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
  assert.equal(nodeB!.lines[4], "status idle")
  assert.equal(layout.endpoints[0]?.entryNodeId, "node-a")
})

test("buildWorkflowGraphLayout applies live selection metadata to the active agent node", () => {
  const layout = buildWorkflowGraphLayout({
    workflow: workflow(),
    agents: [
      agent("agent-a"),
      agent("agent-b", { provider: "opencode", model: "openai/gpt-5.4", effort: "high" }),
    ],
    workflowRuns: [],
    selectedNodeId: "node-b",
  })
  const nodeB = layout.nodes.find((node) => node.id === "node-b")
  assert.ok(nodeB)
  assert.equal(nodeB!.lines[1], "provider opencode")
  assert.equal(nodeB!.lines[2], "model openai/gpt-5.4")
  assert.equal(nodeB!.lines[3], "effort high")
  assert.equal(nodeB!.lines[4], "status idle")
})

test("buildWorkflowGraphLayout uses per-agent effort when present", () => {
  const layout = buildWorkflowGraphLayout({
    workflow: workflow(),
    agents: [
      agent("agent-a"),
      agent("agent-b", { provider: "opencode", model: "openai/gpt-5.4", effort: "high" }),
    ],
    workflowRuns: [],
    selectedNodeId: "node-b",
  })
  const nodeB = layout.nodes.find((node) => node.id === "node-b")
  assert.ok(nodeB)
  assert.equal(nodeB!.lines[1], "provider opencode")
  assert.equal(nodeB!.lines[2], "model openai/gpt-5.4")
  assert.equal(nodeB!.lines[3], "effort high")
  assert.equal(nodeB!.lines[4], "status idle")
})

test("buildWorkflowGraphLayout surfaces the newest active workflow run and node statuses", () => {
  const completedRun = workflowRun({
    id: "run-old",
    status: "Completed",
    created_at_ms: 5,
    node_runs: [
      {
        id: "node-run-old-a",
        node_id: "node-a",
        agent_id: "agent-a",
        status: "Completed",
        summary: "done",
        created_at_ms: 1,
        started_at_ms: 2,
        completed_at_ms: 3,
      },
    ],
  })
  const activeRun = workflowRun({
    id: "run-new",
    status: "Running",
    created_at_ms: 20,
  })
  const layout = buildWorkflowGraphLayout({
    workflow: workflow(),
    agents: [agent("agent-a"), agent("agent-b"), agent("agent-c")],
    workflowRuns: [completedRun, activeRun],
    selectedNodeId: "node-b",
  })
  const nodeA = layout.nodes.find((node) => node.id === "node-a")
  const nodeB = layout.nodes.find((node) => node.id === "node-b")
  const nodeC = layout.nodes.find((node) => node.id === "node-c")
  assert.equal(layout.workflowRunId, "run-new")
  assert.equal(layout.workflowRunStatus, "Running")
  assert.equal(nodeA?.lines[4], "status completed")
  assert.equal(nodeB?.lines[4], "status running")
  assert.equal(nodeC?.lines[4], "status idle")
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

test("routeWorkflowEdge offsets reciprocal edges into separate lanes", () => {
  const leftNode = layoutNode("left", 10, 10)
  const rightNode = layoutNode("right", 60, 10)

  const forward = routeWorkflowEdge(leftNode, rightNode, { reciprocalLane: -1 })
  const reverse = routeWorkflowEdge(rightNode, leftNode, { reciprocalLane: 1 })

  assert.notDeepEqual(forward, reverse)
  assert.notEqual(forward[0]?.y, reverse[0]?.y)
  assert.notEqual(forward[forward.length - 1]?.y, reverse[reverse.length - 1]?.y)
  assert.equal(forward[0]?.x, 39)
  assert.equal(reverse[0]?.x, 60)
})

test("routeWorkflowEdge avoids unrelated node boxes by using orthogonal corners", () => {
  const fromNode = layoutNode("from", 10, 10)
  const blocker = layoutNode("blocker", 45, 8, 18, 10)
  const toNode = layoutNode("to", 80, 10)

  const path = routeWorkflowEdge(fromNode, toNode, { obstacles: [fromNode, blocker, toNode] })

  assert.ok(path.length >= 4)
  assert.ok(path.some((point) => point.y < blocker.y || point.y > blocker.y + blocker.height - 1))
  for (let index = 0; index < path.length - 1; index += 1) {
    const start = path[index]!
    const end = path[index + 1]!
    const intersectsBlocker = segmentIntersectsNode(start, end, blocker)
    assert.equal(intersectsBlocker, false)
  }
})

test("buildWorkflowGraphLayout reorders nodes within a rank to reduce crossings", () => {
  const crossingWorkflow: WorkflowDefinition = {
    id: "flow-crossing",
    alias: null,
    nodes: [
      { id: "node-root", agent_id: "agent-root" },
      { id: "node-a", agent_id: "agent-a" },
      { id: "node-b", agent_id: "agent-b" },
      { id: "node-c", agent_id: "agent-c" },
      { id: "node-d", agent_id: "agent-d" },
    ],
    edges: [
      { id: "edge-root-a", from_node_id: "node-root", to_node_id: "node-a" },
      { id: "edge-root-b", from_node_id: "node-root", to_node_id: "node-b" },
      { id: "edge-a", from_node_id: "node-a", to_node_id: "node-d" },
      { id: "edge-b", from_node_id: "node-b", to_node_id: "node-c" },
    ],
    endpoints: [],
  }

  const layout = buildWorkflowGraphLayout({
    workflow: crossingWorkflow,
    agents: [agent("agent-root"), agent("agent-a"), agent("agent-b"), agent("agent-c"), agent("agent-d")],
    workflowRuns: [],
    selectedNodeId: null,
  })

  const nodeC = layout.nodes.find((node) => node.id === "node-c")
  const nodeD = layout.nodes.find((node) => node.id === "node-d")
  assert.ok(nodeC)
  assert.ok(nodeD)
  assert.ok(nodeD!.x < nodeC!.x, `expected node-d (${nodeD!.x}) left of node-c (${nodeC!.x})`)
})

test("buildWorkflowGraphLayout separates reciprocal edges so both remain visible", () => {
  const reciprocalWorkflow: WorkflowDefinition = {
    id: "flow-reciprocal",
    alias: null,
    nodes: [
      { id: "node-a", agent_id: "agent-a" },
      { id: "node-b", agent_id: "agent-b" },
    ],
    edges: [
      { id: "edge-forward", from_node_id: "node-a", to_node_id: "node-b" },
      { id: "edge-reverse", from_node_id: "node-b", to_node_id: "node-a" },
    ],
    endpoints: [],
  }

  const layout = buildWorkflowGraphLayout({
    workflow: reciprocalWorkflow,
    agents: [agent("agent-a"), agent("agent-b")],
    workflowRuns: [],
    selectedNodeId: null,
  })

  const forward = layout.edges.find((edge) => edge.id === "edge-forward")
  const reverse = layout.edges.find((edge) => edge.id === "edge-reverse")

  assert.ok(forward)
  assert.ok(reverse)
  assert.notDeepEqual(forward!.points, reverse!.points)
  assert.notEqual(forward!.points[0]?.y, reverse!.points[0]?.y)
  assert.notEqual(
    forward!.points[forward!.points.length - 1]?.y,
    reverse!.points[reverse!.points.length - 1]?.y,
  )
})

test("workflow graph drill suite keeps representative topologies clean", () => {
  for (const drillCase of buildWorkflowGraphDrillCases()) {
    const layout = buildWorkflowGraphLayout({
      workflow: drillCase.workflow,
      agents: drillCase.agents,
      workflowRuns: [],
      selectedNodeId: null,
    })
    const validation = validateWorkflowGraphLayout(layout)
    assert.deepEqual(
      validation,
      {
        nodeOverlaps: [],
        diagonalSegments: [],
        edgeNodeCollisions: [],
        reciprocalOverlaps: [],
      },
      `expected clean layout for ${drillCase.id}`,
    )
  }
})

function segmentIntersectsNode(
  start: { x: number; y: number },
  end: { x: number; y: number },
  node: WorkflowGraphNodeLayout,
) {
  const minX = node.x
  const maxX = node.x + node.width - 1
  const minY = node.y
  const maxY = node.y + node.height - 1
  if (start.x === end.x) {
    if (start.x < minX || start.x > maxX) {
      return false
    }
    const segmentMinY = Math.min(start.y, end.y)
    const segmentMaxY = Math.max(start.y, end.y)
    return segmentMaxY >= minY && segmentMinY <= maxY
  }
  if (start.y === end.y) {
    if (start.y < minY || start.y > maxY) {
      return false
    }
    const segmentMinX = Math.min(start.x, end.x)
    const segmentMaxX = Math.max(start.x, end.x)
    return segmentMaxX >= minX && segmentMinX <= maxX
  }
  return false
}

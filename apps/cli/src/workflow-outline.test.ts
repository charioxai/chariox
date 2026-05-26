import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { buildWorkflowGraphDrillCases } from "./workflow-graph-drills.js"
import { buildWorkflowOutline } from "./workflow-outline/build.js"
import { renderWorkflowOutlineToText } from "./workflow-outline/text.js"

function agent(id: string, overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "openai/gpt-5.4",
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
      { id: "node-a", agent_id: "agent-a", instructions: "inspect diff" },
      { id: "node-b", agent_id: "agent-b" },
      { id: "node-c", agent_id: "agent-c" },
    ],
    edges: [
      { id: "edge-a", from_node_id: "node-a", to_node_id: "node-b" },
      { id: "edge-b", from_node_id: "node-b", to_node_id: "node-a" },
      { id: "edge-c", from_node_id: "node-a", to_node_id: "node-c" },
    ],
    endpoints: [
      { id: "endpoint-a", alias: "start", entry_node_id: "node-a" },
    ],
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
        status: "Running",
        summary: null,
        created_at_ms: 1,
        started_at_ms: 2,
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

test("buildWorkflowOutline preserves workflow node order and always includes graph refs", () => {
  const outline = buildWorkflowOutline({
    workflow: workflow(),
    agents: [
      agent("agent-a", { agent_ref: "a1" }),
      agent("agent-b", { agent_ref: "b1" }),
      agent("agent-c", { agent_ref: "c1" }),
    ],
    workflowRuns: [],
    selectedNodeId: "node-b",
  })

  assert.deepEqual(outline.nodes.map((node) => node.id), ["node-a", "node-b", "node-c"])
  assert.deepEqual(outline.nodes[0]?.outgoingEdges.map((edge) => edge.id), ["edge-a", "edge-c"])
  assert.deepEqual(outline.nodes[0]?.incomingEdges.map((edge) => edge.id), ["edge-b"])
  assert.deepEqual(outline.nodes[0]?.entryEndpoints.map((endpoint) => endpoint.id), ["endpoint-a"])
})

test("renderWorkflowOutlineToText keeps graph structure visible while expanding selected node details", () => {
  const outline = buildWorkflowOutline({
    workflow: workflow(),
    agents: [
      agent("agent-a", { agent_ref: "a1", alias: "lead", effort: "high" }),
      agent("agent-b", { agent_ref: "b1", effort: "low" }),
      agent("agent-c", { agent_ref: "c1" }),
    ],
    workflowRuns: [workflowRun()],
    selectedNodeId: "node-a",
  })

  const rendered = renderWorkflowOutlineToText(outline)

  assert.match(rendered, /node node-a • agent a1 \(lead\)/)
  assert.match(rendered, /entry endpoints 1/)
  assert.match(rendered, /endpoint-a \(start\)/)
  assert.match(rendered, /edge-a -> node-b • agent b1/)
  assert.match(rendered, /edge-b <- node-b • agent b1/)
  assert.match(rendered, /provider opencode/)
  assert.match(rendered, /model openai\/gpt-5\.4/)
  assert.match(rendered, /effort high/)
  assert.match(rendered, /status running/)
  assert.match(rendered, /instructions\n  inspect diff/)
})

test("workflow outline redacts collaborator-owned agent ids", () => {
  const outline = buildWorkflowOutline({
    workflow: workflow(),
    agents: [
      agent("agent-a", { agent_ref: "a1", alias: "lead" }),
    ],
    workflowRuns: [],
    selectedNodeId: "node-a",
  })

  const rendered = renderWorkflowOutlineToText(outline)

  assert.match(rendered, /node node-b • agent another collaborator's agent/)
  assert.match(rendered, /edge-c -> node-c • agent another collaborator's agent/)
  assert.doesNotMatch(rendered, /agent-b/)
  assert.doesNotMatch(rendered, /agent-c/)
})

test("workflow outline surfaces failure counts and selected-node failure details", () => {
  const outline = buildWorkflowOutline({
    workflow: workflow(),
    agents: [
      agent("agent-a", { agent_ref: "a1", alias: "lead", effort: "high" }),
      agent("agent-b", { agent_ref: "b1", effort: "low" }),
      agent("agent-c", { agent_ref: "c1" }),
    ],
    workflowRuns: [workflowRun({
      failure_events: [
        {
          kind: "OutputValidationFailed",
          source_node_run_id: "node-run-a",
          edge_ids: ["edge-a"],
          message: "output.message must be an object with ok=true",
          timestamp_ms: 100,
        },
      ],
    })],
    selectedNodeId: "node-a",
  })

  const rendered = renderWorkflowOutlineToText(outline)

  assert.match(rendered, /run: run-1 • status running • failures 1/)
  assert.match(rendered, /failures 1/)
  assert.match(rendered, /recent failure events/)
  assert.match(rendered, /outputvalidationfailed • output\.message must be an object with ok=true/i)
})

test("non-selected workflow nodes omit non-graph attributes in the outline text", () => {
  const outline = buildWorkflowOutline({
    workflow: workflow(),
    agents: [
      agent("agent-a", { agent_ref: "a1", effort: "high" }),
      agent("agent-b", { agent_ref: "b1", effort: "low" }),
      agent("agent-c", { agent_ref: "c1" }),
    ],
    workflowRuns: [workflowRun()],
    selectedNodeId: "node-a",
  })

  const rendered = renderWorkflowOutlineToText(outline)
  const nodeBSection = rendered.split("\n\n").find((section) => section.startsWith("node node-b"))

  assert.ok(nodeBSection)
  assert.match(nodeBSection!, /edge-b -> node-a • agent a1/)
  assert.match(nodeBSection!, /edge-a <- node-a • agent a1/)
  assert.doesNotMatch(nodeBSection!, /provider /)
  assert.doesNotMatch(nodeBSection!, /model /)
  assert.doesNotMatch(nodeBSection!, /effort /)
  assert.doesNotMatch(nodeBSection!, /status /)
  assert.doesNotMatch(nodeBSection!, /instructions/)
})

test("workflow outline drill catalog covers multiple graph shapes with all refs visible", () => {
  for (const drillCase of buildWorkflowGraphDrillCases()) {
    const outline = buildWorkflowOutline({
      workflow: drillCase.workflow,
      agents: drillCase.agents,
      workflowRuns: [],
      selectedNodeId: drillCase.workflow.nodes?.[0]?.id ?? null,
    })
    const rendered = renderWorkflowOutlineToText(outline)

    for (const node of drillCase.workflow.nodes ?? []) {
      assert.match(rendered, new RegExp(`node ${node.id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`), drillCase.id)
    }
    for (const edge of drillCase.workflow.edges ?? []) {
      assert.match(rendered, new RegExp(`\\b${edge.id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`), drillCase.id)
      assert.match(rendered, new RegExp(`\\b${edge.from_node_id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`), drillCase.id)
      assert.match(rendered, new RegExp(`\\b${edge.to_node_id.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`), drillCase.id)
    }
  }
})

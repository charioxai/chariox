import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import {
  handleWorkflowEdgeCommand,
  handleWorkflowEdgeShorthandCommand,
  hasWorkflowEdgeShorthandArgs,
  type WorkflowEdgeCommandContext,
  type WorkflowEdgeCommandDeps,
} from "./workflow-edge-command-handlers.js"

test("workflow edge add resolves agent refs to workflow nodes", async () => {
  const harness = createHarness({
    workflow: workflow({
      nodes: [
        node({ id: "node-a", agent_id: "agent-a" }),
        node({ id: "node-b", agent_id: "agent-b" }),
      ],
    }),
    agentsByRef: {
      alice: agent({ id: "agent-a", alias: "alice" }),
    },
  })

  await handleWorkflowEdgeCommand(harness.deps, harness.context, ["edge", "add", "workflow-1", "alice", "node-b"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "add:workflow-1:node-a:node-b",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:added workflow edge edge-1",
  ])
})

test("workflow edge add rejects duplicate and ambiguous selected-workflow syntax", async () => {
  const duplicate = createHarness({
    workflow: workflow({
      nodes: [node({ id: "node-a" }), node({ id: "node-b" })],
      edges: [edge({ from_node_id: "node-a", to_node_id: "node-b" })],
    }),
  })
  await handleWorkflowEdgeCommand(duplicate.deps, duplicate.context, ["edge", "add", "workflow-1", "node-a", "node-b"])

  const ambiguous = createHarness({
    knownWorkflowRefs: new Set(["workflow-2"]),
    selectedWorkflowRef: "workflow-1",
  })
  await handleWorkflowEdgeCommand(ambiguous.deps, ambiguous.context, ["edge", "add", "workflow-2", "node-b"])

  assert.deepEqual(duplicate.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "footer:error:workflow edge already exists between those nodes",
  ])
  assert.deepEqual(ambiguous.calls, [
    "footer:error:usage: /workflow edge add [workflow-ref] <from-node-id|from-agent-ref> <to-node-id|to-agent-ref> [--handoff-schema <schema-ref>]",
  ])
})

test("workflow edge remove uses the selected workflow by default", async () => {
  const harness = createHarness({ selectedWorkflowRef: "workflow-1" })

  await handleWorkflowEdgeCommand(harness.deps, harness.context, ["edge", "remove", "edge-1"])

  assert.deepEqual(harness.calls, [
    "remove:workflow-1:edge-1",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:removed workflow edge edge-1",
  ])
})

test("workflow edge shorthand creates the edge and opens the workflow screen", async () => {
  const harness = createHarness()

  assert.equal(hasWorkflowEdgeShorthandArgs(["workflow-1", "node-a", "node-b"]), true)
  assert.equal(hasWorkflowEdgeShorthandArgs(["workflow-1", "alias"]), false)

  await handleWorkflowEdgeShorthandCommand(harness.deps, ["workflow-1", "node-a", "node-b"])

  assert.deepEqual(harness.calls, [
    "resolve:workflow-1",
    "upsert:workflow-1",
    "add:workflow-1:node-a:node-b",
    "apply:session-1",
    "select:workflow-1",
    "show",
    "footer:info:added workflow edge edge-1",
  ])
})

type HarnessOptions = Partial<WorkflowEdgeCommandDeps> & {
  agentsByRef?: Record<string, AgentInstance>
  context?: Partial<WorkflowEdgeCommandContext>
  knownWorkflowRefs?: Set<string>
  selectedWorkflowRef?: string | null
  workflow?: WorkflowDefinition
}

function createHarness(overrides: HarnessOptions = {}) {
  const {
    agentsByRef = {},
    context: contextOverrides,
    knownWorkflowRefs = new Set<string>(),
    selectedWorkflowRef = "workflow-1",
    workflow: currentWorkflow = workflow({
      nodes: [node({ id: "node-a" }), node({ id: "node-b" })],
    }),
    ...depOverrides
  } = overrides
  const calls: string[] = []
  const deps: WorkflowEdgeCommandDeps = {
    resolveSessionAgent: (reference) => ({
      agent: reference ? agentsByRef[reference] ?? null : null,
      error: reference && agentsByRef[reference] ? null : `agent '${reference}' not found`,
    }),
    resolveWorkflow: async (workflowRef) => {
      calls.push(`resolve:${workflowRef}`)
      return { workflow: { ...currentWorkflow, id: workflowRef } }
    },
    upsertWorkflowDefinition: (nextWorkflow) => {
      calls.push(`upsert:${nextWorkflow.id}`)
    },
    addWorkflowEdge: async (workflowRef, fromNodeId, toNodeId, handoffSchemaRef) => {
      calls.push(`add:${workflowRef}:${fromNodeId}:${toNodeId}${handoffSchemaRef ? `:${handoffSchemaRef}` : ""}`)
      return {
        edge: edge({ from_node_id: fromNodeId, to_node_id: toNodeId }),
        workflow: workflow({ id: workflowRef }),
        session: session(),
      }
    },
    removeWorkflowEdge: async (workflowRef, edgeId) => {
      calls.push(`remove:${workflowRef}:${edgeId}`)
      return {
        edge: edge({ id: edgeId }),
        workflow: workflow({ id: workflowRef }),
        session: session(),
      }
    },
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    showWorkflowScreen: () => {
      calls.push("show")
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...depOverrides,
  }
  const context: WorkflowEdgeCommandContext = {
    isKnownWorkflowReference: (reference) => Boolean(reference && knownWorkflowRefs.has(reference)),
    selectedWorkflowRef: () => selectedWorkflowRef,
    workflowRefOrSelected: (workflowRef) => workflowRef ?? selectedWorkflowRef,
    ...contextOverrides,
  }
  return { calls, context, deps }
}

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-a",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: null,
    provider: "codex",
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
    ...overrides,
  }
}

function workflow(overrides: Partial<WorkflowDefinition> = {}): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    ...overrides,
  }
}

function node(overrides: Partial<WorkflowNodeDefinition> = {}): WorkflowNodeDefinition {
  return {
    id: "node-a",
    agent_id: "agent-a",
    ...overrides,
  }
}

function edge(overrides: Partial<WorkflowEdgeDefinition> = {}): WorkflowEdgeDefinition {
  return {
    id: "edge-1",
    from_node_id: "node-a",
    to_node_id: "node-b",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 4,
    agents: [],
    config_state: { version: 1, values: {} },
    ...overrides,
  }
}

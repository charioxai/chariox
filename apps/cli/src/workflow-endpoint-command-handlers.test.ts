import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
} from "./cli-types.js"
import {
  handleWorkflowEndpointCommand,
  type WorkflowEndpointCommandContext,
  type WorkflowEndpointCommandDeps,
} from "./workflow-endpoint-command-handlers.js"

test("workflow endpoint new creates an endpoint for the selected workflow", async () => {
  const harness = createHarness({
    selectedWorkflowRef: "workflow-1",
    createWorkflowEndpoint: async (workflowRef, entryNodeId, alias) => {
      harness.calls.push(`create:${workflowRef}:${entryNodeId}:${alias ?? "null"}`)
      return payload({ workflowId: workflowRef, endpoint: endpoint({ id: "endpoint-1", alias: alias ?? null }) })
    },
  })

  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "new", "node-1", "start"])

  assert.deepEqual(harness.calls, [
    "create:workflow-1:node-1:start",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:created workflow endpoint endpoint-1",
  ])
})

test("workflow endpoint alias and bind support explicit workflow refs", async () => {
  const harness = createHarness({
    assignWorkflowEndpointAlias: async (workflowRef, endpointRef, alias) => {
      harness.calls.push(`alias:${workflowRef}:${endpointRef}:${alias}`)
      return payload({ workflowId: workflowRef, endpoint: endpoint({ id: endpointRef, alias }) })
    },
    bindWorkflowEndpoint: async (workflowRef, endpointRef, entryNodeId) => {
      harness.calls.push(`bind:${workflowRef}:${endpointRef}:${entryNodeId}`)
      return payload({ workflowId: workflowRef, endpoint: endpoint({ id: endpointRef, entry_node_id: entryNodeId }) })
    },
  })

  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "alias", "workflow-1", "endpoint-1", "ship"])
  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "bind", "workflow-1", "endpoint-1", "node-2"])

  assert.deepEqual(harness.calls, [
    "alias:workflow-1:endpoint-1:ship",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:workflow endpoint endpoint-1 aliased as ship",
    "bind:workflow-1:endpoint-1:node-2",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:workflow endpoint endpoint-1 bound to node node-2",
  ])
})

test("workflow endpoint rebinds and removes endpoints from the selected workflow", async () => {
  const harness = createHarness({
    bindWorkflowEndpoint: async (workflowRef, endpointRef, entryNodeId) => {
      harness.calls.push(`bind:${workflowRef}:${endpointRef}:${entryNodeId}`)
      return payload({ workflowId: workflowRef, endpoint: endpoint({ id: endpointRef, entry_node_id: entryNodeId }) })
    },
    removeWorkflowEndpoint: async (workflowRef, endpointRef) => {
      harness.calls.push(`remove:${workflowRef}:${endpointRef}`)
      return payload({ workflowId: workflowRef, endpoint: endpoint({ id: endpointRef }) })
    },
  })

  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "rebind", "endpoint-1", "node-2"])
  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "remove", "endpoint-1"])

  assert.deepEqual(harness.calls, [
    "bind:workflow-1:endpoint-1:node-2",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:workflow endpoint endpoint-1 rebound to node node-2",
    "remove:workflow-1:endpoint-1",
    "apply:session-1",
    "select:workflow-1",
    "footer:info:removed workflow endpoint endpoint-1",
  ])
})

test("workflow endpoint command validates action usage", async () => {
  const harness = createHarness({ selectedWorkflowRef: null })

  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "new"])
  await handleWorkflowEndpointCommand(harness.deps, harness.context, ["endpoint", "unknown"])

  assert.deepEqual(harness.calls, [
    "footer:error:usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias]",
    "footer:error:usage: /workflow endpoint new [workflow-ref] <entry-node-id> [alias] | alias [workflow-ref] <endpoint-ref> <alias> | bind|rebind [workflow-ref] <endpoint-ref> <entry-node-id> | remove [workflow-ref] <endpoint-ref>",
  ])
})

type HarnessOptions = Partial<WorkflowEndpointCommandDeps> & {
  context?: Partial<WorkflowEndpointCommandContext>
  selectedWorkflowRef?: string | null
}

function createHarness(overrides: HarnessOptions) {
  const { context: contextOverrides, selectedWorkflowRef = "workflow-1", ...depOverrides } = overrides
  const calls: string[] = []
  const deps: WorkflowEndpointCommandDeps = {
    createWorkflowEndpoint: async (workflowRef, _entryNodeId, alias) => (
      payload({ workflowId: workflowRef, endpoint: endpoint({ alias: alias ?? null }) })
    ),
    assignWorkflowEndpointAlias: async (workflowRef, endpointRef, alias) => (
      payload({ workflowId: workflowRef, endpoint: endpoint({ id: endpointRef, alias }) })
    ),
    bindWorkflowEndpoint: async (workflowRef, endpointRef, entryNodeId) => (
      payload({ workflowId: workflowRef, endpoint: endpoint({ id: endpointRef, entry_node_id: entryNodeId }) })
    ),
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
    },
    selectWorkflowCanvas: (workflowId) => {
      calls.push(`select:${workflowId}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`footer:${tone}:${message}`)
    },
    ...depOverrides,
  }
  const context: WorkflowEndpointCommandContext = {
    firstWorkflowArgIsExplicit: (workflowRef) => Boolean(workflowRef && workflowRef.startsWith("workflow-")),
    workflowRefOrSelected: (workflowRef) => workflowRef ?? selectedWorkflowRef,
    ...contextOverrides,
  }
  return { calls, context, deps }
}

function payload(input: {
  workflowId: string
  endpoint: WorkflowEndpointDefinition
}) {
  return {
    endpoint: input.endpoint,
    workflow: workflow({ id: input.workflowId }),
    session: session(),
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

function endpoint(overrides: Partial<WorkflowEndpointDefinition> = {}): WorkflowEndpointDefinition {
  return {
    id: "endpoint-1",
    alias: null,
    entry_node_id: "node-1",
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
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

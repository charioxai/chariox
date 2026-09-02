import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import { createWorkflowTopologyController } from "./workflow-topology-controller.js"

test("workflow topology controller creates endpoints through design ops", async () => {
  const current = workflow()
  const updated = { ...current, endpoints: [endpoint("endpoint-test", "node-1")] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: current } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: { ...session(), workflows: [updated] } } },
  })

  const payload = await harness.controller.createWorkflowEndpoint("workflow-1", "node-1", "start")

  assert.equal(payload.endpoint.id, "endpoint-test")
  assert.deepEqual(harness.designOps, [{
    kind: "endpoint_add",
    workflow_id: "workflow-1",
    endpoint: { id: "endpoint-test", entry_node_id: "node-1", alias: "start" },
  }])
})

test("workflow topology controller updates node runtime settings", async () => {
  const current = workflow()
  const updatedNode = { ...node("node-1"), max_turns: 5, wait_for_all_inputs: true }
  const updated = { ...current, nodes: [updatedNode, node("node-2")] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: current } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: { ...session(), workflows: [updated] } } },
  })

  const payload = await harness.controller.setWorkflowNodeMaxTurns("workflow-1", "node-1", 5)
  const waitPayload = await harness.controller.setWorkflowNodeWaitForAllInputs("workflow-1", "node-1", true)

  assert.equal(payload.node.id, "node-1")
  assert.equal(waitPayload.node.id, "node-1")
  assert.deepEqual(harness.designOps, [{
    kind: "node_update",
    workflow_id: "workflow-1",
    node_id: "node-1",
    patch: { max_turns: 5 },
  }, {
    kind: "node_update",
    workflow_id: "workflow-1",
    node_id: "node-1",
    patch: { wait_for_all_inputs: true },
  }])
})

test("workflow topology controller adds graph edges", async () => {
  const current = workflow()
  const updated = { ...current, edges: [edge("edge-test")] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: current } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: { ...session(), workflows: [updated] } } },
  })

  const payload = await harness.controller.addWorkflowEdge("workflow-1", "node-1", "node-2")

  assert.equal(payload.edge.id, "edge-test")
  assert.deepEqual(harness.designOps, [{
    kind: "edge_add",
    workflow_id: "workflow-1",
    edge: { id: "edge-test", from_node_id: "node-1", to_node_id: "node-2" },
  }])
})

test("workflow topology controller sets endpoint capacity through design ops", async () => {
  const current = { ...workflow(), endpoints: [endpoint("endpoint-1", "node-1")] }
  const updated = { ...current, endpoints: [endpointWithCapacity("endpoint-1", "node-1", 4)] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: current } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: { ...session(), workflows: [updated] } } },
  })

  const payload = await harness.controller.setWorkflowEndpointMaxInstances("workflow-1", "endpoint-1", 4)

  assert.equal(payload.endpoint.max_instances, 4)
  assert.deepEqual(harness.designOps, [{
    kind: "endpoint_update",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    patch: { max_instances: 4 },
  }])
})

test("workflow topology controller resolves and removes endpoint aliases through design ops", async () => {
  const current = {
    ...workflow(),
    endpoints: [endpoint("endpoint-1", "node-1")],
  }
  current.endpoints[0]!.alias = "start"
  const next = { ...current, endpoints: [] }
  const nextSession = { ...session(), workflows: [next] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: current } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: nextSession } },
  })

  const payload = await harness.controller.removeWorkflowEndpoint("workflow-1", "start")

  assert.equal(payload.endpoint.id, "endpoint-1")
  assert.deepEqual(payload.workflow.endpoints, [])
  assert.deepEqual(harness.designOps, [{
    kind: "endpoint_remove",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
  }])
})

function createHarness(responses: Record<string, Record<string, unknown>>) {
  const requests: Record<string, unknown>[] = []
  const designOps: unknown[] = []
  const controller = createWorkflowTopologyController({
    sessionId: () => "session-1",
    createWorkflowDesignId: (prefix) => `${prefix}-test`,
    sendRequest: async (request) => {
      requests.push(request)
      const variant = Object.keys(request)[0] ?? ""
      return responses[variant] ?? {}
    },
    applyWorkflowDesignOp: async (op) => {
      designOps.push(op)
      const payload = responses.ApplyWorkflowDesignOp?.WorkflowDesignOpAccepted as { session: RuntimeSession } | undefined
      if (!payload) throw new Error("missing ApplyWorkflowDesignOp response")
      return payload
    },
  })
  return { controller, requests, designOps }
}

function session(): RuntimeSession {
  return { id: "session-1" } as RuntimeSession
}

function workflow(): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: "Workflow",
    nodes: [node("node-1"), node("node-2")],
  }
}

function endpoint(id: string, entryNodeId: string): WorkflowEndpointDefinition {
  return {
    id,
    alias: null,
    entry_node_id: entryNodeId,
  }
}

function endpointWithCapacity(id: string, entryNodeId: string, maxInstances: number): WorkflowEndpointDefinition {
  return { ...endpoint(id, entryNodeId), max_instances: maxInstances }
}

function node(id: string): WorkflowNodeDefinition {
  return {
    id,
    agent_id: "agent-1",
  }
}

function edge(id: string): WorkflowEdgeDefinition {
  return {
    id,
    from_node_id: "node-1",
    to_node_id: "node-2",
  }
}

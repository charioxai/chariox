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

test("workflow topology controller creates endpoints through the kernel request path", async () => {
  const harness = createHarness({
    CreateWorkflowEndpoint: {
      WorkflowEndpointCreated: {
        endpoint: endpoint("endpoint-1", "node-1"),
        workflow: workflow(),
        session: session(),
      },
    },
  })

  const payload = await harness.controller.createWorkflowEndpoint("workflow-1", "node-1", "start")

  assert.equal(payload.endpoint.id, "endpoint-1")
  assert.deepEqual(harness.requests, [
    {
      CreateWorkflowEndpoint: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
        entry_node_id: "node-1",
        alias: "start",
      },
    },
  ])
})

test("workflow topology controller updates node runtime settings", async () => {
  const harness = createHarness({
    SetWorkflowNodeMaxTurns: {
      WorkflowNodeMaxTurnsUpdated: {
        node: node("node-1"),
        workflow: workflow(),
        session: session(),
      },
    },
  })

  const payload = await harness.controller.setWorkflowNodeMaxTurns("workflow-1", "node-1", 5)

  assert.equal(payload.node.id, "node-1")
  assert.deepEqual(harness.requests.at(-1), {
    SetWorkflowNodeMaxTurns: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      node_id: "node-1",
      max_turns: 5,
    },
  })
})

test("workflow topology controller adds graph edges", async () => {
  const harness = createHarness({
    AddWorkflowEdge: {
      WorkflowEdgeAdded: {
        edge: edge("edge-1"),
        workflow: workflow(),
        session: session(),
      },
    },
  })

  const payload = await harness.controller.addWorkflowEdge("workflow-1", "node-1", "node-2")

  assert.equal(payload.edge.id, "edge-1")
  assert.deepEqual(harness.requests.at(-1), {
    AddWorkflowEdge: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      from_node_id: "node-1",
      to_node_id: "node-2",
    },
  })
})

function createHarness(responses: Record<string, Record<string, unknown>>) {
  const requests: Record<string, unknown>[] = []
  const controller = createWorkflowTopologyController({
    sessionId: () => "session-1",
    sendRequest: async (request) => {
      requests.push(request)
      const variant = Object.keys(request)[0] ?? ""
      return responses[variant] ?? {}
    },
  })
  return { controller, requests }
}

function session(): RuntimeSession {
  return { id: "session-1" } as RuntimeSession
}

function workflow(): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: "Workflow",
    nodes: [],
  }
}

function endpoint(id: string, entryNodeId: string): WorkflowEndpointDefinition {
  return {
    id,
    alias: null,
    entry_node_id: entryNodeId,
  }
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

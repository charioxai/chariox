import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowDefinition } from "./cli-types.js"
import { workflowsWithDesignOp } from "./workflow-design-op-state.js"

test("workflow design op state applies workflow lifecycle patches", () => {
  const created = workflowsWithDesignOp([], {
    kind: "workflow_create",
    workflow: { id: "workflow-1", alias: "Review" },
  })

  assert.deepEqual(created, [{
    id: "workflow-1",
    alias: "Review",
    nodes: [],
    edges: [],
    endpoints: [],
  }])

  const duplicate = workflowsWithDesignOp(created, {
    kind: "workflow_create",
    workflow: { id: "workflow-1", alias: "Ignored" },
  })
  assert.equal(duplicate.length, 1)
  assert.equal(duplicate[0]?.alias, "Review")

  const updated = workflowsWithDesignOp(created, {
    kind: "workflow_update",
    workflow_id: "workflow-1",
    patch: {
      alias: "Release",
      flush_agent_context_before_run: true,
      run_output_schema_ref: "final",
    },
  })
  assert.equal(updated[0]?.alias, "Release")
  assert.equal(updated[0]?.flush_agent_context_before_run, true)
  assert.equal(updated[0]?.run_output_schema_ref, "final")

  assert.deepEqual(workflowsWithDesignOp(updated, {
    kind: "workflow_remove",
    workflow_id: "workflow-1",
  }), [])
})

test("workflow design op state applies node edge and endpoint patches", () => {
  const initial: WorkflowDefinition[] = [{
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
  }]

  const withNode = workflowsWithDesignOp(initial, {
    kind: "node_add",
    workflow_id: "workflow-1",
    node: {
      id: "node-1",
      agent_id: "agent-1",
      instructions: "Review",
      can_complete_workflow_run: true,
      max_turns: 3,
    },
  })
  assert.deepEqual(withNode[0]?.nodes?.map((node) => [node.id, node.agent_id, node.instructions, node.max_turns]), [
    ["node-1", "agent-1", "Review", 3],
  ])

  const nodeUpdated = workflowsWithDesignOp(withNode, {
    kind: "node_update",
    workflow_id: "workflow-1",
    node_id: "node-1",
    patch: { instructions: "Updated", max_turns: null },
  })
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.instructions, "Updated")
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.max_turns, null)

  const withEndpoint = workflowsWithDesignOp(nodeUpdated, {
    kind: "endpoint_add",
    workflow_id: "workflow-1",
    endpoint: { id: "endpoint-1", alias: "default", entry_node_id: "node-1" },
  })
  const endpointUpdated = workflowsWithDesignOp(withEndpoint, {
    kind: "endpoint_update",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    patch: { alias: "release" },
  })
  assert.deepEqual(endpointUpdated[0]?.endpoints, [{
    id: "endpoint-1",
    alias: "release",
    entry_node_id: "node-1",
  }])

  const withEdge = workflowsWithDesignOp(endpointUpdated, {
    kind: "edge_add",
    workflow_id: "workflow-1",
    edge: {
      id: "edge-1",
      from_node_id: "node-1",
      to_node_id: "node-2",
      source_side: "right",
      target_side: "left",
      handoff_schema_ref: "handoff",
      validation_policy: "warn",
    },
  })
  assert.deepEqual(withEdge[0]?.edges, [{
    id: "edge-1",
    from_node_id: "node-1",
    to_node_id: "node-2",
    source_side: "right",
    target_side: "left",
    handoff_schema_ref: "handoff",
    validation_policy: "warn",
  }])

  const edgeUpdated = workflowsWithDesignOp(withEdge, {
    kind: "edge_update",
    workflow_id: "workflow-1",
    edge_id: "edge-1",
    patch: { validation_policy: "halt" },
  })
  assert.equal(edgeUpdated[0]?.edges?.[0]?.validation_policy, "halt")

  const nodeRemoved = workflowsWithDesignOp(edgeUpdated, {
    kind: "node_remove",
    workflow_id: "workflow-1",
    node_id: "node-1",
  })
  assert.deepEqual(nodeRemoved[0]?.nodes, [])
  assert.deepEqual(nodeRemoved[0]?.edges, [])
  assert.deepEqual(nodeRemoved[0]?.endpoints, [])
})

import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowDefinition } from "./cli-types.js"
import { workflowsWithDesignOp } from "./workflow-design-op-state.js"

test("workflow design op state applies workflow lifecycle patches", () => {
  const created = workflowsWithDesignOp([], {
    kind: "workflow_create",
    workflow: {
      id: "workflow-1",
      alias: "Review",
      prompt: "Review the change",
      flush_agent_context_before_run: false,
      max_concurrent: 3,
      run_output_schema_ref: "final",
      schemas: [{ id: "final", alias: "Final", description: "Final output", schema: { type: "object" } }],
    },
  })

  assert.deepEqual(created, [{
    id: "workflow-1",
    alias: "Review",
    prompt: "Review the change",
    flush_agent_context_before_run: false,
    max_concurrent: 3,
    run_output_schema_ref: "final",
    schemas: [{ id: "final", alias: "Final", description: "Final output", schema: { type: "object" } }],
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
      prompt: null,
      flush_agent_context_before_run: true,
      max_concurrent: 5,
      run_output_schema_ref: null,
    },
  })
  assert.equal(updated[0]?.alias, "Release")
  assert.equal(updated[0]?.prompt, null)
  assert.equal(updated[0]?.flush_agent_context_before_run, true)
  assert.equal(updated[0]?.max_concurrent, 5)
  assert.equal(updated[0]?.run_output_schema_ref, null)

  assert.deepEqual(workflowsWithDesignOp(updated, {
    kind: "workflow_remove",
    workflow_id: "workflow-1",
  }), [])
})

test("workflow design op state applies schema lifecycle patches", () => {
  const initial: WorkflowDefinition[] = [{
    id: "workflow-1",
    alias: null,
    schemas: [{ id: "existing", alias: "Old", description: "Old schema", schema: { type: "string" } }],
    nodes: [],
    edges: [],
    endpoints: [],
  }]

  const added = workflowsWithDesignOp(initial, {
    kind: "schema_add",
    workflow_id: "workflow-1",
    schema: { id: "handoff", alias: "Handoff", description: null, schema: { type: "object" } },
  })
  assert.deepEqual(added[0]?.schemas, [
    { id: "existing", alias: "Old", description: "Old schema", schema: { type: "string" } },
    { id: "handoff", alias: "Handoff", description: null, schema: { type: "object" } },
  ])

  const updated = workflowsWithDesignOp(added, {
    kind: "schema_update",
    workflow_id: "workflow-1",
    schema_id: "handoff",
    patch: { alias: null, description: "Updated", schema: { type: "array" } },
  })
  assert.deepEqual(updated[0]?.schemas?.[1], {
    id: "handoff",
    alias: null,
    description: "Updated",
    schema: { type: "array" },
  })

  const removed = workflowsWithDesignOp(updated, {
    kind: "schema_remove",
    workflow_id: "workflow-1",
    schema_id: "existing",
  })
  assert.deepEqual(removed[0]?.schemas?.map((schema) => schema.id), ["handoff"])
  assert.equal(initial[0]?.schemas?.[0]?.alias, "Old")
})

test("workflow design op state applies node edge and endpoint patches", () => {
  const initial: WorkflowDefinition[] = [{
    id: "workflow-1",
    alias: null,
    nodes: [],
    edges: [],
    endpoints: [],
    canvas_layout: {
      version: 1,
      revision: 4,
      coordinate_space: "workflow-canvas-v1",
      nodes: {},
      endpoints: {},
      exits: { "node-1": { x: 90, y: 10 } },
      edges: {},
    },
  }]

  const withNode = workflowsWithDesignOp(initial, {
    kind: "node_add",
    workflow_id: "workflow-1",
    node: {
      id: "node-1",
      agent_id: "agent-1",
      label: "Reviewer",
      instructions: "Review",
      can_complete_workflow_run: true,
      can_emit_intermediate_run_output: true,
      wait_for_all_inputs: true,
      intermediate_output_schema_ref: "intermediate",
      max_turns: 3,
    },
    position: { x: 10, y: 20 },
  })
  assert.deepEqual(withNode[0]?.nodes, [{
    id: "node-1",
    agent_id: "agent-1",
    public_label: "Reviewer",
    instructions: "Review",
    can_complete_workflow_run: true,
    can_emit_intermediate_run_output: true,
    wait_for_all_inputs: true,
    intermediate_output_schema_ref: "intermediate",
    max_turns: 3,
  }])
  assert.deepEqual(withNode[0]?.canvas_layout?.nodes?.["node-1"], { x: 10, y: 20 })

  const withFallbackLabel = workflowsWithDesignOp(initial, {
    kind: "node_add",
    workflow_id: "workflow-1",
    node: { id: "node-2", agent_id: "agent-2" },
  })
  assert.equal(withFallbackLabel[0]?.nodes?.[0]?.public_label, "agent-2")

  const nodeUpdated = workflowsWithDesignOp(withNode, {
    kind: "node_update",
    workflow_id: "workflow-1",
    node_id: "node-1",
    patch: {
      label: "Verifier",
      instructions: null,
      can_complete_workflow_run: false,
      can_emit_intermediate_run_output: false,
      wait_for_all_inputs: false,
      intermediate_output_schema_ref: null,
      max_turns: null,
    },
  })
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.public_label, "Verifier")
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.instructions, null)
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.can_complete_workflow_run, false)
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.can_emit_intermediate_run_output, false)
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.wait_for_all_inputs, false)
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.intermediate_output_schema_ref, null)
  assert.equal(nodeUpdated[0]?.nodes?.[0]?.max_turns, null)
  assert.equal(nodeUpdated[0]?.canvas_layout?.exits?.["node-1"], undefined)

  const nodeMoved = workflowsWithDesignOp(nodeUpdated, {
    kind: "node_move",
    workflow_id: "workflow-1",
    node_id: "node-1",
    position: { x: 30, y: 40 },
  })
  assert.deepEqual(nodeMoved[0]?.canvas_layout?.nodes?.["node-1"], { x: 30, y: 40 })

  const withEndpoint = workflowsWithDesignOp(nodeMoved, {
    kind: "endpoint_add",
    workflow_id: "workflow-1",
    endpoint: { id: "endpoint-1", alias: "default", entry_node_id: "node-1" },
    position: { x: -10, y: 40 },
  })
  const endpointUpdated = workflowsWithDesignOp(withEndpoint, {
    kind: "endpoint_update",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    patch: { alias: "release", entry_node_id: "node-2" },
  })
  assert.deepEqual(endpointUpdated[0]?.endpoints, [{
    id: "endpoint-1",
    alias: "release",
    entry_node_id: "node-2",
  }])

  const endpointRebound = workflowsWithDesignOp(endpointUpdated, {
    kind: "endpoint_update",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    patch: { entry_node_id: "node-1" },
  })

  const endpointMoved = workflowsWithDesignOp(endpointRebound, {
    kind: "endpoint_move",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    position: { x: -20, y: 50 },
  })
  assert.deepEqual(endpointMoved[0]?.canvas_layout?.endpoints?.["endpoint-1"], { x: -20, y: 50 })

  const withEdge = workflowsWithDesignOp(endpointMoved, {
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
    patch: { handoff_schema_ref: null, validation_policy: "halt" },
  })
  assert.equal(edgeUpdated[0]?.edges?.[0]?.handoff_schema_ref, null)
  assert.equal(edgeUpdated[0]?.edges?.[0]?.validation_policy, "halt")

  const edgeWithLayout: WorkflowDefinition[] = [{
    ...edgeUpdated[0]!,
    canvas_layout: {
      ...edgeUpdated[0]!.canvas_layout!,
      edges: { "edge-1": { waypoints: [{ x: 1, y: 2 }] } },
    },
  }]
  const edgeRemoved = workflowsWithDesignOp(edgeWithLayout, {
    kind: "edge_remove",
    workflow_id: "workflow-1",
    edge_id: "edge-1",
  })
  assert.deepEqual(edgeRemoved[0]?.edges, [])
  assert.equal(edgeRemoved[0]?.canvas_layout?.edges?.["edge-1"], undefined)

  const endpointRemoved = workflowsWithDesignOp(withEdge, {
    kind: "endpoint_remove",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
  })
  assert.deepEqual(endpointRemoved[0]?.endpoints, [])
  assert.equal(endpointRemoved[0]?.canvas_layout?.endpoints?.["endpoint-1"], undefined)

  const nodeRemoved = workflowsWithDesignOp(edgeWithLayout, {
    kind: "node_remove",
    workflow_id: "workflow-1",
    node_id: "node-1",
  })
  assert.deepEqual(nodeRemoved[0]?.nodes, [])
  assert.deepEqual(nodeRemoved[0]?.edges, [])
  assert.deepEqual(nodeRemoved[0]?.endpoints, [])
  assert.equal(nodeRemoved[0]?.canvas_layout?.nodes?.["node-1"], undefined)
  assert.equal(nodeRemoved[0]?.canvas_layout?.exits?.["node-1"], undefined)
  assert.equal(nodeRemoved[0]?.canvas_layout?.edges?.["edge-1"], undefined)
  assert.equal(nodeRemoved[0]?.canvas_layout?.endpoints?.["endpoint-1"], undefined)
})

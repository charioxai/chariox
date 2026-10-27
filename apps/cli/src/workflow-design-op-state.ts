import type { WorkflowDesignOp } from "@chariox/kernel-client/kernel-types"

import type {
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowSchemaDefinition,
} from "./cli-types.js"

export function workflowsWithDesignOp(
  current: readonly WorkflowDefinition[],
  op: WorkflowDesignOp,
): WorkflowDefinition[] {
  const workflows = current.map((workflow) => ({ ...workflow }))
  const replaceWorkflow = (
    workflowId: string,
    update: (workflow: WorkflowDefinition) => WorkflowDefinition,
  ) => {
    const index = workflows.findIndex((workflow) => workflow.id === workflowId)
    if (index >= 0) {
      workflows[index] = update(workflows[index]!)
    }
  }

  switch (op.kind) {
    case "workflow_create": {
      if (!workflows.some((workflow) => workflow.id === op.workflow.id)) {
        const workflow: WorkflowDefinition = {
          id: op.workflow.id,
          alias: op.workflow.alias ?? null,
          schemas: (op.workflow.schemas ?? []).map(copyWorkflowSchema),
          nodes: [],
          edges: [],
          endpoints: [],
        }
        if (Object.prototype.hasOwnProperty.call(op.workflow, "prompt")) workflow.prompt = op.workflow.prompt ?? null
        if (typeof op.workflow.flush_agent_context_before_run === "boolean") {
          workflow.flush_agent_context_before_run = op.workflow.flush_agent_context_before_run
        }
        if (typeof op.workflow.max_concurrent === "number") workflow.max_concurrent = op.workflow.max_concurrent
        if (Object.prototype.hasOwnProperty.call(op.workflow, "run_output_schema_ref")) {
          workflow.run_output_schema_ref = op.workflow.run_output_schema_ref ?? null
        }
        workflows.push(workflow)
      }
      break
    }
    case "workflow_update":
      replaceWorkflow(op.workflow_id, (workflow) => workflowWithPatch(workflow, op.patch))
      break
    case "workflow_remove":
      return workflows.filter((workflow) => workflow.id !== op.workflow_id)
    case "schema_add":
      replaceWorkflow(op.workflow_id, (workflow) => {
        const schemas = workflow.schemas ?? []
        return {
          ...workflow,
          schemas: schemas.some((schema) => schema.id === op.schema.id)
            ? schemas
            : [...schemas, copyWorkflowSchema(op.schema)],
        }
      })
      break
    case "schema_update":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        schemas: (workflow.schemas ?? []).map((schema) => schema.id === op.schema_id
          ? workflowSchemaWithPatch(schema, op.patch)
          : schema),
      }))
      break
    case "schema_remove":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        schemas: (workflow.schemas ?? []).filter((schema) => schema.id !== op.schema_id),
      }))
      break
    case "node_add":
      replaceWorkflow(op.workflow_id, (workflow) => {
        const nodes = workflow.nodes?.some((node) => node.id === op.node.id)
          ? workflow.nodes
          : [...(workflow.nodes ?? []), workflowNodeFromDesign(op.node)]
        return {
          ...workflow,
          nodes,
          ...(op.position ? { canvas_layout: canvasLayoutWithPosition(workflow.canvas_layout, "nodes", op.node.id, op.position) } : {}),
        }
      })
      break
    case "node_update":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        nodes: (workflow.nodes ?? []).map((node) => node.id === op.node_id
          ? workflowNodeWithPatch(node, op.patch)
          : node),
        ...(op.patch.can_complete_workflow_run === false
          ? { canvas_layout: canvasLayoutWithoutKey(workflow.canvas_layout, "exits", op.node_id) }
          : {}),
      }))
      break
    case "node_move":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        canvas_layout: canvasLayoutWithPosition(workflow.canvas_layout, "nodes", op.node_id, op.position),
      }))
      break
    case "node_remove":
      replaceWorkflow(op.workflow_id, workflowWithoutNode(op.node_id))
      break
    case "edge_add":
      replaceWorkflow(op.workflow_id, (workflow) => {
        const edges = workflow.edges?.some((edge) => edge.id === op.edge.id)
          ? workflow.edges
          : [...(workflow.edges ?? []), workflowEdgeFromDesign(op.edge)]
        return { ...workflow, edges }
      })
      break
    case "edge_update":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        edges: (workflow.edges ?? []).map((edge) => edge.id === op.edge_id
          ? workflowEdgeWithPatch(edge, op.patch)
          : edge),
      }))
      break
    case "edge_remove":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        edges: (workflow.edges ?? []).filter((edge) => edge.id !== op.edge_id),
        canvas_layout: canvasLayoutWithoutKey(workflow.canvas_layout, "edges", op.edge_id),
      }))
      break
    case "endpoint_add":
      replaceWorkflow(op.workflow_id, (workflow) => {
        const endpoints = workflow.endpoints?.some((endpoint) => endpoint.id === op.endpoint.id)
          ? workflow.endpoints
          : [...(workflow.endpoints ?? []), workflowEndpointFromDesign(op.endpoint)]
        return {
          ...workflow,
          endpoints,
          ...(op.position ? { canvas_layout: canvasLayoutWithPosition(workflow.canvas_layout, "endpoints", op.endpoint.id, op.position) } : {}),
        }
      })
      break
    case "endpoint_update":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        endpoints: (workflow.endpoints ?? []).map((endpoint) => endpoint.id === op.endpoint_id
          ? workflowEndpointWithPatch(endpoint, op.patch)
          : endpoint),
      }))
      break
    case "endpoint_move":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        canvas_layout: canvasLayoutWithPosition(workflow.canvas_layout, "endpoints", op.endpoint_id, op.position),
      }))
      break
    case "endpoint_remove":
      replaceWorkflow(op.workflow_id, (workflow) => ({
        ...workflow,
        endpoints: (workflow.endpoints ?? []).filter((endpoint) => endpoint.id !== op.endpoint_id),
        canvas_layout: canvasLayoutWithoutKey(workflow.canvas_layout, "endpoints", op.endpoint_id),
      }))
      break
    default: {
      const exhaustive: never = op
      void exhaustive
      break
    }
  }

  return workflows
}

function workflowWithPatch(
  workflow: WorkflowDefinition,
  patch: Extract<WorkflowDesignOp, { kind: "workflow_update" }>["patch"],
): WorkflowDefinition {
  const next: WorkflowDefinition = { ...workflow }
  if (Object.prototype.hasOwnProperty.call(patch, "alias")) {
    next.alias = patch.alias ?? null
  }
  if (Object.prototype.hasOwnProperty.call(patch, "prompt")) {
    next.prompt = patch.prompt ?? null
  }
  if (typeof patch.flush_agent_context_before_run === "boolean") {
    next.flush_agent_context_before_run = patch.flush_agent_context_before_run
  }
  if (typeof patch.max_concurrent === "number") next.max_concurrent = patch.max_concurrent
  if (Object.prototype.hasOwnProperty.call(patch, "run_output_schema_ref")) {
    next.run_output_schema_ref = patch.run_output_schema_ref ?? null
  }
  return next
}

function copyWorkflowSchema(schema: WorkflowSchemaDefinition): WorkflowSchemaDefinition {
  return { ...schema }
}

function workflowSchemaWithPatch(
  schema: WorkflowSchemaDefinition,
  patch: Extract<WorkflowDesignOp, { kind: "schema_update" }>["patch"],
): WorkflowSchemaDefinition {
  const next = copyWorkflowSchema(schema)
  if (Object.prototype.hasOwnProperty.call(patch, "alias")) next.alias = patch.alias ?? null
  if (Object.prototype.hasOwnProperty.call(patch, "description")) next.description = patch.description ?? null
  if (Object.prototype.hasOwnProperty.call(patch, "schema")) next.schema = patch.schema
  return next
}

function workflowNodeFromDesign(node: Extract<WorkflowDesignOp, { kind: "node_add" }>["node"]): WorkflowNodeDefinition {
  const next: WorkflowNodeDefinition = {
    id: node.id,
    agent_id: node.agent_id,
    public_label: node.label ?? node.agent_id,
  }
  if (Object.prototype.hasOwnProperty.call(node, "instructions")) next.instructions = node.instructions ?? null
  if (Object.prototype.hasOwnProperty.call(node, "can_complete_workflow_run")) next.can_complete_workflow_run = node.can_complete_workflow_run ?? false
  if (Object.prototype.hasOwnProperty.call(node, "can_emit_intermediate_run_output")) next.can_emit_intermediate_run_output = node.can_emit_intermediate_run_output ?? false
  if (Object.prototype.hasOwnProperty.call(node, "wait_for_all_inputs")) next.wait_for_all_inputs = node.wait_for_all_inputs ?? false
  if (Object.prototype.hasOwnProperty.call(node, "intermediate_output_schema_ref")) next.intermediate_output_schema_ref = node.intermediate_output_schema_ref ?? null
  if (Object.prototype.hasOwnProperty.call(node, "max_turns")) next.max_turns = node.max_turns ?? null
  return next
}

function workflowNodeWithPatch(
  node: WorkflowNodeDefinition,
  patch: Extract<WorkflowDesignOp, { kind: "node_update" }>["patch"],
): WorkflowNodeDefinition {
  const next: WorkflowNodeDefinition = { ...node }
  if (typeof patch.label === "string") next.public_label = patch.label
  if (Object.prototype.hasOwnProperty.call(patch, "instructions")) next.instructions = patch.instructions ?? null
  if (Object.prototype.hasOwnProperty.call(patch, "can_complete_workflow_run")) next.can_complete_workflow_run = patch.can_complete_workflow_run ?? false
  if (Object.prototype.hasOwnProperty.call(patch, "can_emit_intermediate_run_output")) next.can_emit_intermediate_run_output = patch.can_emit_intermediate_run_output ?? false
  if (Object.prototype.hasOwnProperty.call(patch, "wait_for_all_inputs")) next.wait_for_all_inputs = patch.wait_for_all_inputs ?? false
  if (Object.prototype.hasOwnProperty.call(patch, "intermediate_output_schema_ref")) next.intermediate_output_schema_ref = patch.intermediate_output_schema_ref ?? null
  if (Object.prototype.hasOwnProperty.call(patch, "max_turns")) next.max_turns = patch.max_turns ?? null
  return next
}

function workflowWithoutNode(nodeId: string) {
  return (workflow: WorkflowDefinition): WorkflowDefinition => {
    const removedEdgeIds = (workflow.edges ?? [])
      .filter((edge) => edge.from_node_id === nodeId || edge.to_node_id === nodeId)
      .map((edge) => edge.id)
    const removedEndpointIds = (workflow.endpoints ?? [])
      .filter((endpoint) => endpoint.entry_node_id === nodeId)
      .map((endpoint) => endpoint.id)
    let canvasLayout = canvasLayoutWithoutKey(workflow.canvas_layout, "nodes", nodeId)
    canvasLayout = canvasLayoutWithoutKey(canvasLayout, "exits", nodeId)
    for (const edgeId of removedEdgeIds) canvasLayout = canvasLayoutWithoutKey(canvasLayout, "edges", edgeId)
    for (const endpointId of removedEndpointIds) canvasLayout = canvasLayoutWithoutKey(canvasLayout, "endpoints", endpointId)
    return {
      ...workflow,
      nodes: (workflow.nodes ?? []).filter((node) => node.id !== nodeId),
      edges: (workflow.edges ?? []).filter((edge) => edge.from_node_id !== nodeId && edge.to_node_id !== nodeId),
      endpoints: (workflow.endpoints ?? []).filter((endpoint) => endpoint.entry_node_id !== nodeId),
      canvas_layout: canvasLayout,
    }
  }
}

type WorkflowCanvasMap = "nodes" | "endpoints" | "exits"
type WorkflowCanvasCollection = WorkflowCanvasMap | "edges"

function canvasLayoutWithPosition(
  current: WorkflowDefinition["canvas_layout"],
  collection: WorkflowCanvasMap,
  id: string,
  position: { x: number; y: number },
) {
  const layout = copyCanvasLayout(current)
  layout[collection] = { ...(layout[collection] ?? {}), [id]: { ...position } }
  layout.revision += 1
  return layout
}

function canvasLayoutWithoutKey(
  current: WorkflowDefinition["canvas_layout"],
  collection: WorkflowCanvasCollection,
  id: string,
) {
  if (!current || !current[collection] || !Object.prototype.hasOwnProperty.call(current[collection], id)) return current ?? null
  const layout = copyCanvasLayout(current)
  if (collection === "edges") {
    const entries = { ...layout.edges }
    delete entries[id]
    layout.edges = entries
  } else if (collection === "nodes") {
    const entries = { ...layout.nodes }
    delete entries[id]
    layout.nodes = entries
  } else if (collection === "endpoints") {
    const entries = { ...layout.endpoints }
    delete entries[id]
    layout.endpoints = entries
  } else {
    const entries = { ...layout.exits }
    delete entries[id]
    layout.exits = entries
  }
  layout.revision += 1
  return layout
}

function copyCanvasLayout(current: NonNullable<WorkflowDefinition["canvas_layout"]> | null | undefined) {
  return {
    version: current?.version ?? 1,
    revision: current?.revision ?? 0,
    coordinate_space: current?.coordinate_space ?? "workflow-canvas-v1",
    nodes: { ...(current?.nodes ?? {}) },
    endpoints: { ...(current?.endpoints ?? {}) },
    exits: { ...(current?.exits ?? {}) },
    edges: { ...(current?.edges ?? {}) },
  }
}

function workflowEdgeFromDesign(edge: Extract<WorkflowDesignOp, { kind: "edge_add" }>["edge"]): WorkflowEdgeDefinition {
  const next: WorkflowEdgeDefinition = {
    id: edge.id,
    from_node_id: edge.from_node_id,
    to_node_id: edge.to_node_id,
    handoff_schema_ref: edge.handoff_schema_ref ?? null,
    validation_policy: edge.validation_policy ?? null,
  }
  if (Object.prototype.hasOwnProperty.call(edge, "source_side")) next.source_side = edge.source_side ?? null
  if (Object.prototype.hasOwnProperty.call(edge, "target_side")) next.target_side = edge.target_side ?? null
  return next
}

function workflowEdgeWithPatch(
  edge: WorkflowEdgeDefinition,
  patch: Extract<WorkflowDesignOp, { kind: "edge_update" }>["patch"],
): WorkflowEdgeDefinition {
  return {
    ...edge,
    ...(Object.prototype.hasOwnProperty.call(patch, "handoff_schema_ref") ? { handoff_schema_ref: patch.handoff_schema_ref ?? null } : {}),
    ...(Object.prototype.hasOwnProperty.call(patch, "validation_policy") ? { validation_policy: patch.validation_policy ?? null } : {}),
  }
}

function workflowEndpointFromDesign(endpoint: Extract<WorkflowDesignOp, { kind: "endpoint_add" }>["endpoint"]): WorkflowEndpointDefinition {
  return {
    id: endpoint.id,
    alias: endpoint.alias ?? null,
    entry_node_id: endpoint.entry_node_id,
  }
}

function workflowEndpointWithPatch(
  endpoint: WorkflowEndpointDefinition,
  patch: Extract<WorkflowDesignOp, { kind: "endpoint_update" }>["patch"],
): WorkflowEndpointDefinition {
  return {
    ...endpoint,
    ...(Object.prototype.hasOwnProperty.call(patch, "alias") ? { alias: patch.alias ?? null } : {}),
    ...(Object.prototype.hasOwnProperty.call(patch, "entry_node_id") && patch.entry_node_id ? { entry_node_id: patch.entry_node_id } : {}),
  }
}

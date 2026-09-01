import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import type { WorkflowDesignNodePatch, WorkflowDesignOp } from "@chariox/kernel-client/kernel-types"
import { resolveWorkflowRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowTopologyControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowDesignOp: (op: WorkflowDesignOp) => Promise<{ session: RuntimeSession }>
  createWorkflowDesignId: (prefix: string) => string
}

export function createWorkflowTopologyController(deps: WorkflowTopologyControllerDeps) {
  const createWorkflowEndpoint = async (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    workflowNode(workflow, entryNodeId)
    const endpointId = deps.createWorkflowDesignId("endpoint")
    const payload = await deps.applyWorkflowDesignOp({
      kind: "endpoint_add",
      workflow_id: workflow.id,
      endpoint: { id: endpointId, entry_node_id: entryNodeId, alias: alias ?? null },
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { endpoint: workflowEndpoint(updatedWorkflow, endpointId), workflow: updatedWorkflow, session: payload.session }
  }

  const assignWorkflowEndpointAlias = async (
    workflowRef: string,
    endpointRef: string,
    alias: string,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    const endpoint = resolveWorkflowEndpoint(workflow, endpointRef)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "endpoint_update",
      workflow_id: workflow.id,
      endpoint_id: endpoint.id,
      patch: { alias },
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { endpoint: workflowEndpoint(updatedWorkflow, endpoint.id), workflow: updatedWorkflow, session: payload.session }
  }

  const bindWorkflowEndpoint = async (
    workflowRef: string,
    endpointRef: string,
    entryNodeId: string,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    const endpoint = resolveWorkflowEndpoint(workflow, endpointRef)
    workflowNode(workflow, entryNodeId)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "endpoint_update",
      workflow_id: workflow.id,
      endpoint_id: endpoint.id,
      patch: { entry_node_id: entryNodeId },
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { endpoint: workflowEndpoint(updatedWorkflow, endpoint.id), workflow: updatedWorkflow, session: payload.session }
  }

  const setWorkflowEndpointMaxInstances = async (
    workflowRef: string,
    endpointRef: string,
    maxInstances: number,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    const endpoint = resolveWorkflowEndpoint(workflow, endpointRef)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "endpoint_update",
      workflow_id: workflow.id,
      endpoint_id: endpoint.id,
      patch: { max_instances: maxInstances },
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { endpoint: workflowEndpoint(updatedWorkflow, endpoint.id), workflow: updatedWorkflow, session: payload.session }
  }

  const removeWorkflowEndpoint = async (
    workflowRef: string,
    endpointRef: string,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    const endpoint = resolveWorkflowEndpoint(workflow, endpointRef)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "endpoint_remove",
      workflow_id: workflow.id,
      endpoint_id: endpoint.id,
    })
    return { endpoint, workflow: workflowFromSession(payload.session, workflow.id), session: payload.session }
  }

  const addWorkflowNode = async (workflowRef: string, agentId: string) => {
    const workflow = await resolveWorkflow(workflowRef)
    if (workflow.nodes?.some((node) => node.agent_id === agentId)) {
      throw new Error(`workflow already has a node for agent ${agentId}`)
    }
    const nodeId = deps.createWorkflowDesignId("node")
    const payload = await deps.applyWorkflowDesignOp({
      kind: "node_add",
      workflow_id: workflow.id,
      node: { id: nodeId, agent_id: agentId },
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { node: workflowNode(updatedWorkflow, nodeId), workflow: updatedWorkflow, session: payload.session }
  }

  const removeWorkflowNode = async (workflowRef: string, nodeId: string) => {
    const workflow = await resolveWorkflow(workflowRef)
    const node = workflowNode(workflow, nodeId)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "node_remove",
      workflow_id: workflow.id,
      node_id: node.id,
    })
    return { node, workflow: workflowFromSession(payload.session, workflow.id), session: payload.session }
  }

  const updateWorkflowNodeInstructions = async (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => {
    return updateWorkflowNode(workflowRef, nodeId, { instructions })
  }

  const setWorkflowNodeCanCompleteRun = async (
    workflowRef: string,
    nodeId: string,
    canCompleteWorkflowRun: boolean,
  ) => {
    return updateWorkflowNode(workflowRef, nodeId, { can_complete_workflow_run: canCompleteWorkflowRun })
  }

  const setWorkflowNodeMaxTurns = async (
    workflowRef: string,
    nodeId: string,
    maxTurns: number | null,
  ) => {
    return updateWorkflowNode(workflowRef, nodeId, { max_turns: maxTurns })
  }

  const setWorkflowNodeCanEmitIntermediateOutput = async (
    workflowRef: string,
    nodeId: string,
    canEmitIntermediateWorkflowRunOutput: boolean,
  ) => {
    return updateWorkflowNode(workflowRef, nodeId, { can_emit_intermediate_run_output: canEmitIntermediateWorkflowRunOutput })
  }

  const setWorkflowNodeWaitForAllInputs = async (
    workflowRef: string,
    nodeId: string,
    waitForAllInputs: boolean,
  ) => {
    return updateWorkflowNode(workflowRef, nodeId, { wait_for_all_inputs: waitForAllInputs })
  }

  const setWorkflowNodeIntermediateOutputSchema = async (
    workflowRef: string,
    nodeId: string,
    intermediateOutputSchemaRef: string | null,
  ) => {
    return updateWorkflowNode(workflowRef, nodeId, { intermediate_output_schema_ref: intermediateOutputSchemaRef })
  }

  const addWorkflowEdge = async (
    workflowRef: string,
    fromNodeId: string,
    toNodeId: string,
    handoffSchemaRef?: string | null,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    workflowNode(workflow, fromNodeId)
    workflowNode(workflow, toNodeId)
    const edgeId = deps.createWorkflowDesignId("edge")
    const payload = await deps.applyWorkflowDesignOp({
      kind: "edge_add",
      workflow_id: workflow.id,
      edge: {
        id: edgeId,
        from_node_id: fromNodeId,
        to_node_id: toNodeId,
        ...(handoffSchemaRef ? { handoff_schema_ref: handoffSchemaRef } : {}),
      },
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { edge: workflowEdge(updatedWorkflow, edgeId), workflow: updatedWorkflow, session: payload.session }
  }

  const removeWorkflowEdge = async (workflowRef: string, edgeId: string) => {
    const workflow = await resolveWorkflow(workflowRef)
    const edge = workflowEdge(workflow, edgeId)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "edge_remove",
      workflow_id: workflow.id,
      edge_id: edge.id,
    })
    return { edge, workflow: workflowFromSession(payload.session, workflow.id), session: payload.session }
  }

  const resolveWorkflow = async (workflowRef: string): Promise<WorkflowDefinition> => {
    const response = await deps.sendRequest(resolveWorkflowRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved").workflow
  }

  const updateWorkflowNode = async (
    workflowRef: string,
    nodeId: string,
    patch: WorkflowDesignNodePatch,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    workflowNode(workflow, nodeId)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "node_update",
      workflow_id: workflow.id,
      node_id: nodeId,
      patch,
    })
    const updatedWorkflow = workflowFromSession(payload.session, workflow.id)
    return { node: workflowNode(updatedWorkflow, nodeId), workflow: updatedWorkflow, session: payload.session }
  }

  return {
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    setWorkflowEndpointMaxInstances,
    removeWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeWaitForAllInputs,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    addWorkflowEdge,
    removeWorkflowEdge,
  }
}

function workflowFromSession(session: RuntimeSession, workflowId: string): WorkflowDefinition {
  const workflow = session.workflows?.find((candidate) => candidate.id === workflowId)
  if (!workflow) throw new Error(`workflow design response did not include workflow ${workflowId}`)
  return workflow
}

function workflowNode(workflow: WorkflowDefinition, nodeId: string): WorkflowNodeDefinition {
  const node = workflow.nodes?.find((candidate) => candidate.id === nodeId)
  if (!node) throw new Error(`workflow node '${nodeId}' not found`)
  return node
}

function workflowEdge(workflow: WorkflowDefinition, edgeId: string): WorkflowEdgeDefinition {
  const edge = workflow.edges?.find((candidate) => candidate.id === edgeId)
  if (!edge) throw new Error(`workflow edge '${edgeId}' not found`)
  return edge
}

function workflowEndpoint(workflow: WorkflowDefinition, endpointId: string): WorkflowEndpointDefinition {
  const endpoint = workflow.endpoints?.find((candidate) => candidate.id === endpointId)
  if (!endpoint) throw new Error(`workflow endpoint '${endpointId}' not found`)
  return endpoint
}

function resolveWorkflowEndpoint(workflow: WorkflowDefinition, endpointRef: string): WorkflowEndpointDefinition {
  const endpoint = workflow.endpoints?.find((candidate) => (
    candidate.id === endpointRef || candidate.alias === endpointRef
  ))
  if (!endpoint) throw new Error(`workflow endpoint '${endpointRef}' not found`)
  return endpoint
}

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
} from "./cli-types.js"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  aliasWorkflowEndpointRequest,
  bindWorkflowEndpointRequest,
  createWorkflowEndpointRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowNodeMaxTurnsRequest,
  setWorkflowNodeWaitForAllInputsRequest,
  updateWorkflowNodeInstructionsRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowTopologyControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
}

export function createWorkflowTopologyController(deps: WorkflowTopologyControllerDeps) {
  const createWorkflowEndpoint = async (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => {
    const response = await deps.sendRequest(
      createWorkflowEndpointRequest(deps.sessionId(), workflowRef, entryNodeId, alias),
    )
    return expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEndpointCreated",
    )
  }

  const assignWorkflowEndpointAlias = async (
    workflowRef: string,
    endpointRef: string,
    alias: string,
  ) => {
    const response = await deps.sendRequest(
      aliasWorkflowEndpointRequest(deps.sessionId(), workflowRef, endpointRef, alias),
    )
    return expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEndpointAliased",
    )
  }

  const bindWorkflowEndpoint = async (
    workflowRef: string,
    endpointRef: string,
    entryNodeId: string,
  ) => {
    const response = await deps.sendRequest(
      bindWorkflowEndpointRequest(deps.sessionId(), workflowRef, endpointRef, entryNodeId),
    )
    return expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEndpointBound",
    )
  }

  const addWorkflowNode = async (workflowRef: string, agentId: string) => {
    const response = await deps.sendRequest(addWorkflowNodeRequest(deps.sessionId(), workflowRef, agentId))
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeAdded",
    )
  }

  const removeWorkflowNode = async (workflowRef: string, nodeId: string) => {
    const response = await deps.sendRequest(removeWorkflowNodeRequest(deps.sessionId(), workflowRef, nodeId))
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeRemoved",
    )
  }

  const updateWorkflowNodeInstructions = async (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => {
    const response = await deps.sendRequest(
      updateWorkflowNodeInstructionsRequest(deps.sessionId(), workflowRef, nodeId, instructions),
    )
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeInstructionsUpdated",
    )
  }

  const setWorkflowNodeCanCompleteRun = async (
    workflowRef: string,
    nodeId: string,
    canCompleteWorkflowRun: boolean,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowNodeCanCompleteRunRequest(deps.sessionId(), workflowRef, nodeId, canCompleteWorkflowRun),
    )
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeCanCompleteRunUpdated",
    )
  }

  const setWorkflowNodeMaxTurns = async (
    workflowRef: string,
    nodeId: string,
    maxTurns: number | null,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowNodeMaxTurnsRequest(deps.sessionId(), workflowRef, nodeId, maxTurns),
    )
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeMaxTurnsUpdated",
    )
  }

  const setWorkflowNodeCanEmitIntermediateOutput = async (
    workflowRef: string,
    nodeId: string,
    canEmitIntermediateWorkflowRunOutput: boolean,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowNodeCanEmitIntermediateOutputRequest(
        deps.sessionId(),
        workflowRef,
        nodeId,
        canEmitIntermediateWorkflowRunOutput,
      ),
    )
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeCanEmitIntermediateOutputUpdated",
    )
  }

  const setWorkflowNodeWaitForAllInputs = async (
    workflowRef: string,
    nodeId: string,
    waitForAllInputs: boolean,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowNodeWaitForAllInputsRequest(
        deps.sessionId(),
        workflowRef,
        nodeId,
        waitForAllInputs,
      ),
    )
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeWaitForAllInputsUpdated",
    )
  }

  const setWorkflowNodeIntermediateOutputSchema = async (
    workflowRef: string,
    nodeId: string,
    intermediateOutputSchemaRef: string | null,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowNodeIntermediateOutputSchemaRequest(
        deps.sessionId(),
        workflowRef,
        nodeId,
        intermediateOutputSchemaRef,
      ),
    )
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeIntermediateOutputSchemaUpdated",
    )
  }

  const addWorkflowEdge = async (
    workflowRef: string,
    fromNodeId: string,
    toNodeId: string,
    handoffSchemaRef?: string | null,
  ) => {
    const response = await deps.sendRequest(
      addWorkflowEdgeRequest(deps.sessionId(), workflowRef, fromNodeId, toNodeId, handoffSchemaRef),
    )
    return expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEdgeAdded",
    )
  }

  const removeWorkflowEdge = async (workflowRef: string, edgeId: string) => {
    const response = await deps.sendRequest(removeWorkflowEdgeRequest(deps.sessionId(), workflowRef, edgeId))
    return expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEdgeRemoved",
    )
  }

  return {
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
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

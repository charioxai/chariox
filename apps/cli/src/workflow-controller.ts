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
  aliasWorkflowRequest,
  bindWorkflowEndpointRequest,
  createWorkflowEndpointRequest,
  createWorkflowRequest,
  listWorkflowsRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  resolveWorkflowRequest,
} from "./ipc-requests.js"
import { toggleWorkspaceScreenMode, type WorkspaceScreenMode } from "./workspace-screen.js"
import {
  cycleWorkflowNodeId,
  resolveSelectedWorkflow,
  resolveSelectedWorkflowNodeId,
} from "./workflow-graph/index.js"

type WorkflowControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  applySessionState: (session: RuntimeSession) => void
  selectedWorkflowId: () => string | null
  setSelectedWorkflowId: (value: string | null) => void
  selectedWorkflowNodeId: () => string | null
  setSelectedWorkflowNodeId: (value: string | null) => void
  workspaceScreenMode: () => WorkspaceScreenMode
  setWorkspaceScreenMode: (value: WorkspaceScreenMode) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
}

export function deriveWorkflowSelectionState(
  workflows: WorkflowDefinition[],
  selectedWorkflowId: string | null,
  selectedNodeId: string | null,
) {
  const workflow = resolveSelectedWorkflow(workflows, selectedWorkflowId)
  return {
    workflow,
    workflowId: workflow?.id ?? null,
    nodeId: resolveSelectedWorkflowNodeId(workflow, selectedNodeId),
  }
}

export function createWorkflowController(deps: WorkflowControllerDeps) {
  const workflowScreenActive = () => deps.isAttached() && deps.workspaceScreenMode() === "workflow"
  const selectedWorkflow = () => resolveSelectedWorkflow(deps.sessionState().workflows ?? [], deps.selectedWorkflowId())

  const toggleWorkspaceScreen = () => {
    if (!deps.isAttached()) {
      return
    }
    deps.setWorkspaceScreenMode(toggleWorkspaceScreenMode(deps.workspaceScreenMode()))
    deps.rebuildTranscript()
    deps.applyResponseLayout()
  }

  const showWorkflowScreen = () => {
    if (!deps.isAttached() || workflowScreenActive()) {
      return
    }
    deps.setWorkspaceScreenMode("workflow")
    deps.rebuildTranscript()
    deps.applyResponseLayout()
  }

  const selectWorkflowCanvas = (workflowId: string | null) => {
    deps.setSelectedWorkflowId(workflowId)
    deps.setSelectedWorkflowNodeId(null)
    if (workflowScreenActive()) {
      deps.rebuildTranscript()
    }
  }

  const cycleWorkflowCanvasNode = (step = 1) => {
    const nextNodeId = cycleWorkflowNodeId(selectedWorkflow(), deps.selectedWorkflowNodeId(), step)
    if (nextNodeId === deps.selectedWorkflowNodeId()) {
      return
    }
    deps.setSelectedWorkflowNodeId(nextNodeId)
    if (workflowScreenActive()) {
      deps.rebuildTranscript()
    }
  }

  const replaceWorkflowDefinitions = (workflows: WorkflowDefinition[]) => {
    deps.applySessionState({
      ...deps.sessionState(),
      workflows,
    })
  }

  const upsertWorkflowDefinition = (workflow: WorkflowDefinition) => {
    const currentWorkflows = deps.sessionState().workflows ?? []
    const existingIndex = currentWorkflows.findIndex((entry) => entry.id === workflow.id)
    const workflows = existingIndex === -1
      ? [...currentWorkflows, workflow]
      : currentWorkflows.map((entry, index) => (index === existingIndex ? workflow : entry))
    replaceWorkflowDefinitions(workflows)
  }

  const createWorkflow = async (alias?: string | null) => {
    const response = await deps.sendRequest(createWorkflowRequest(deps.sessionState().id, alias))
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowCreated",
    )
    deps.applySessionState(payload.session)
    deps.setSelectedWorkflowId(payload.workflow.id)
    deps.setSelectedWorkflowNodeId(null)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const listWorkflows = async () => {
    const response = await deps.sendRequest(listWorkflowsRequest(deps.sessionState().id))
    const payload = expectVariant<{ workflows: WorkflowDefinition[] }>(response, "WorkflowsListed")
    return payload.workflows
  }

  const resolveWorkflow = async (workflowRef: string) => {
    const response = await deps.sendRequest(resolveWorkflowRequest(deps.sessionState().id, workflowRef))
    return expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved")
  }

  const assignWorkflowAlias = async (workflowId: string, alias: string) => {
    const response = await deps.sendRequest(aliasWorkflowRequest(deps.sessionState().id, workflowId, alias))
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowAliased",
    )
    deps.applySessionState(payload.session)
    if (payload.workflow) {
      deps.rebuildTranscript()
      deps.applyResponseLayout()
    }
    return payload.workflow
  }

  const createWorkflowEndpoint = async (
    workflowRef: string,
    entryNodeId: string,
    alias?: string | null,
  ) => {
    const response = await deps.sendRequest(
      createWorkflowEndpointRequest(deps.sessionState().id, workflowRef, entryNodeId, alias),
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
      aliasWorkflowEndpointRequest(deps.sessionState().id, workflowRef, endpointRef, alias),
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
      bindWorkflowEndpointRequest(deps.sessionState().id, workflowRef, endpointRef, entryNodeId),
    )
    return expectVariant<{ endpoint: WorkflowEndpointDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEndpointBound",
    )
  }

  const addWorkflowNode = async (workflowRef: string, agentId: string) => {
    const response = await deps.sendRequest(addWorkflowNodeRequest(deps.sessionState().id, workflowRef, agentId))
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeAdded",
    )
  }

  const removeWorkflowNode = async (workflowRef: string, nodeId: string) => {
    const response = await deps.sendRequest(removeWorkflowNodeRequest(deps.sessionState().id, workflowRef, nodeId))
    return expectVariant<{ node: WorkflowNodeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowNodeRemoved",
    )
  }

  const addWorkflowEdge = async (workflowRef: string, fromNodeId: string, toNodeId: string) => {
    const response = await deps.sendRequest(
      addWorkflowEdgeRequest(deps.sessionState().id, workflowRef, fromNodeId, toNodeId),
    )
    return expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEdgeAdded",
    )
  }

  const removeWorkflowEdge = async (workflowRef: string, edgeId: string) => {
    const response = await deps.sendRequest(removeWorkflowEdgeRequest(deps.sessionState().id, workflowRef, edgeId))
    return expectVariant<{ edge: WorkflowEdgeDefinition; workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowEdgeRemoved",
    )
  }

  return {
    workflowScreenActive,
    selectedWorkflow,
    toggleWorkspaceScreen,
    showWorkflowScreen,
    selectWorkflowCanvas,
    cycleWorkflowCanvasNode,
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    createWorkflowEndpoint,
    assignWorkflowEndpointAlias,
    bindWorkflowEndpoint,
    addWorkflowNode,
    removeWorkflowNode,
    addWorkflowEdge,
    removeWorkflowEdge,
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

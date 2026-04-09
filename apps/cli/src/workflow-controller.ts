import type {
  QueuedWorkflowLaunch,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEdgeDefinition,
  WorkflowEndpointDefinition,
  WorkflowNodeDefinition,
  WorkflowRun,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  aliasWorkflowEndpointRequest,
  aliasWorkflowRequest,
  bindWorkflowEndpointRequest,
  createWorkflowEndpointRequest,
  createWorkflowRequest,
  createWorkflowWatchdogRequest,
  clearQueuedWorkflowLaunchesRequest,
  cancelWorkflowRunRequest,
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
  listQueuedWorkflowLaunchesRequest,
  listWorkflowWatchdogsRequest,
  listWorkflowsRequest,
  listWorkflowRunsRequest,
  removeQueuedWorkflowLaunchRequest,
  removeWorkflowEdgeRequest,
  removeWorkflowNodeRequest,
  removeWorkflowWatchdogRequest,
  resumeWorkflowRunRequest,
  resolveWorkflowRequest,
  setWorkflowFlushContextRequest,
  setWorkflowIntermediateOutputSchemaRequest,
  setWorkflowLaunchPolicyRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  setWorkflowNodeIntermediateOutputSchemaRequest,
  setWorkflowNodeMaxTurnsRequest,
  setWorkflowRunOutputSchemaRequest,
  setWorkflowWatchdogEnabledRequest,
  updateWorkflowNodeInstructionsRequest,
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

  const updateWorkflowNodeInstructions = async (
    workflowRef: string,
    nodeId: string,
    instructions: string | null,
  ) => {
    const response = await deps.sendRequest(
      updateWorkflowNodeInstructionsRequest(deps.sessionState().id, workflowRef, nodeId, instructions),
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
      setWorkflowNodeCanCompleteRunRequest(deps.sessionState().id, workflowRef, nodeId, canCompleteWorkflowRun),
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
      setWorkflowNodeMaxTurnsRequest(deps.sessionState().id, workflowRef, nodeId, maxTurns),
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
        deps.sessionState().id,
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

  const setWorkflowNodeIntermediateOutputSchema = async (
    workflowRef: string,
    nodeId: string,
    intermediateOutputSchemaRef: string | null,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowNodeIntermediateOutputSchemaRequest(
        deps.sessionState().id,
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

  const invokeWorkflowEndpoint = async (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
  ) => {
    const response = await deps.sendRequest(
      invokeWorkflowEndpointRequest(deps.sessionState().id, workflowRef, endpointRef, prompt),
    )
    if ("WorkflowRunInvoked" in response) {
      const payload = expectVariant<{
        workflow_run: WorkflowRun
        workflow: WorkflowDefinition
        endpoint: WorkflowEndpointDefinition
        session: RuntimeSession
      }>(response, "WorkflowRunInvoked")
      deps.applySessionState(payload.session)
      deps.rebuildTranscript()
      deps.applyResponseLayout()
      return payload
    }
    const payload = expectVariant<{
      queued_launch: QueuedWorkflowLaunch
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
      session: RuntimeSession
    }>(response, "WorkflowRunQueued")
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const createWorkflowWatchdog = async (
    workflowRef: string,
    endpointRef: string,
    intervalSeconds: number,
    invocationPrompt: string,
    policy: "skip" | "queue",
    maxWakeups?: number | null,
  ) => {
    const response = await deps.sendRequest(
      createWorkflowWatchdogRequest(
        deps.sessionState().id,
        workflowRef,
        endpointRef,
        intervalSeconds,
        invocationPrompt,
        policy,
        maxWakeups,
      ),
    )
    const payload = expectVariant<{
      watchdog: WorkflowWatchdogDefinition
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
      session: RuntimeSession
    }>(response, "WorkflowWatchdogCreated")
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const listWorkflowWatchdogs = async (workflowRef?: string | null) => {
    const response = await deps.sendRequest(listWorkflowWatchdogsRequest(deps.sessionState().id, workflowRef))
    return expectVariant<{ watchdogs: WorkflowWatchdogDefinition[] }>(response, "WorkflowWatchdogsListed")
  }

  const setWorkflowWatchdogEnabled = async (watchdogRef: string, enabled: boolean) => {
    const response = await deps.sendRequest(setWorkflowWatchdogEnabledRequest(deps.sessionState().id, watchdogRef, enabled))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogUpdated",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const removeWorkflowWatchdog = async (watchdogRef: string) => {
    const response = await deps.sendRequest(removeWorkflowWatchdogRequest(deps.sessionState().id, watchdogRef))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogRemoved",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const setWorkflowLaunchPolicy = async (policy: "reject" | "queue") => {
    const response = await deps.sendRequest(
      setWorkflowLaunchPolicyRequest(deps.sessionState().id, policy),
    )
    const payload = expectVariant<{ session: RuntimeSession }>(
      response,
      "WorkflowLaunchPolicyUpdated",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const setWorkflowFlushContext = async (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowFlushContextRequest(
        deps.sessionState().id,
        workflowRef,
        flushAgentContextBeforeRun,
      ),
    )
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowFlushContextUpdated",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const setWorkflowRunOutputSchema = async (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowRunOutputSchemaRequest(deps.sessionState().id, workflowRef, runOutputSchemaRef),
    )
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowRunOutputSchemaUpdated",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const setWorkflowIntermediateOutputSchema = async (
    workflowRef: string,
    intermediateOutputSchemaRef: string | null,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowIntermediateOutputSchemaRequest(
        deps.sessionState().id,
        workflowRef,
        intermediateOutputSchemaRef,
      ),
    )
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowIntermediateOutputSchemaUpdated",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const listQueuedWorkflowLaunches = async () => {
    const response = await deps.sendRequest(
      listQueuedWorkflowLaunchesRequest(deps.sessionState().id),
    )
    const payload = expectVariant<{ queued_launches: QueuedWorkflowLaunch[] }>(
      response,
      "QueuedWorkflowLaunchesListed",
    )
    return payload.queued_launches
  }

  const removeQueuedWorkflowLaunch = async (queueItemRef: string) => {
    const response = await deps.sendRequest(
      removeQueuedWorkflowLaunchRequest(deps.sessionState().id, queueItemRef),
    )
    const payload = expectVariant<{ queued_launch: QueuedWorkflowLaunch; session: RuntimeSession }>(
      response,
      "QueuedWorkflowLaunchRemoved",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const clearQueuedWorkflowLaunches = async () => {
    const response = await deps.sendRequest(
      clearQueuedWorkflowLaunchesRequest(deps.sessionState().id),
    )
    const payload = expectVariant<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>(
      response,
      "QueuedWorkflowLaunchesCleared",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const listWorkflowRuns = async (workflowRef?: string | null) => {
    const response = await deps.sendRequest(listWorkflowRunsRequest(deps.sessionState().id, workflowRef))
    const payload = expectVariant<{ workflow_runs: WorkflowRun[] }>(response, "WorkflowRunsListed")
    return payload.workflow_runs
  }

  const getWorkflowRun = async (workflowRunRef: string) => {
    const response = await deps.sendRequest(getWorkflowRunRequest(deps.sessionState().id, workflowRunRef))
    return expectVariant<{ workflow_run: WorkflowRun }>(response, "WorkflowRun")
  }

  const cancelWorkflowRun = async (workflowRunRef: string) => {
    const response = await deps.sendRequest(cancelWorkflowRunRequest(deps.sessionState().id, workflowRunRef))
    const payload = expectVariant<{ workflow_run: WorkflowRun; session: RuntimeSession }>(
      response,
      "WorkflowRunCancelled",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const resumeWorkflowRun = async (workflowRunRef: string) => {
    const response = await deps.sendRequest(resumeWorkflowRunRequest(deps.sessionState().id, workflowRunRef))
    const payload = expectVariant<{ workflow_run: WorkflowRun; session: RuntimeSession }>(
      response,
      "WorkflowRunResumed",
    )
    deps.applySessionState(payload.session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
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
    updateWorkflowNodeInstructions,
    setWorkflowNodeCanCompleteRun,
    setWorkflowNodeCanEmitIntermediateOutput,
    setWorkflowNodeIntermediateOutputSchema,
    setWorkflowNodeMaxTurns,
    addWorkflowEdge,
    removeWorkflowEdge,
    invokeWorkflowEndpoint,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    setWorkflowIntermediateOutputSchema,
    setWorkflowLaunchPolicy,
    listQueuedWorkflowLaunches,
    removeQueuedWorkflowLaunch,
    clearQueuedWorkflowLaunches,
    listWorkflowRuns,
    getWorkflowRun,
    cancelWorkflowRun,
    resumeWorkflowRun,
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

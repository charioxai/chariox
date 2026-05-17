import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import {
  aliasWorkflowRequest,
  createWorkflowRequest,
  createWorkflowWatchdogRequest,
  listWorkflowWatchdogsRequest,
  listWorkflowsRequest,
  removeWorkflowWatchdogRequest,
  resolveWorkflowRequest,
  setWorkflowFlushContextRequest,
  setWorkflowIntermediateOutputSchemaRequest,
  setWorkflowLaunchPolicyRequest,
  setWorkflowRunOutputSchemaRequest,
  setWorkflowWatchdogEnabledRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { WorkspaceScreenMode } from "./workspace-screen.js"
import { createWorkflowRuntimeController } from "./workflow-runtime-controller.js"
import { createWorkflowScreenController } from "./workflow-screen-controller.js"
import { createWorkflowSessionStateController } from "./workflow-session-state.js"
import { createWorkflowTopologyController } from "./workflow-topology-controller.js"

export {
  createWorkflowSelectionSyncController,
  deriveWorkflowSelectionState,
} from "./workflow-selection-sync.js"

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

export function createWorkflowController(deps: WorkflowControllerDeps) {
  const workflowScreen = createWorkflowScreenController({
    isAttached: deps.isAttached,
    workflows: () => deps.sessionState().workflows ?? [],
    selectedWorkflowId: deps.selectedWorkflowId,
    setSelectedWorkflowId: deps.setSelectedWorkflowId,
    selectedWorkflowNodeId: deps.selectedWorkflowNodeId,
    setSelectedWorkflowNodeId: deps.setSelectedWorkflowNodeId,
    workspaceScreenMode: deps.workspaceScreenMode,
    setWorkspaceScreenMode: deps.setWorkspaceScreenMode,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
  })
  const workflowSessionState = createWorkflowSessionStateController({
    sessionState: deps.sessionState,
    applySessionState: deps.applySessionState,
    rebuildTranscript: deps.rebuildTranscript,
    applyResponseLayout: deps.applyResponseLayout,
  })
  const workflowTopology = createWorkflowTopologyController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
  })
  const workflowRuntime = createWorkflowRuntimeController({
    sendRequest: deps.sendRequest,
    sessionId: () => deps.sessionState().id,
    applyWorkflowSessionRefresh: workflowSessionState.applyWorkflowSessionRefresh,
  })

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
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
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
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const removeWorkflowWatchdog = async (watchdogRef: string) => {
    const response = await deps.sendRequest(removeWorkflowWatchdogRequest(deps.sessionState().id, watchdogRef))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogRemoved",
    )
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
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
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
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
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
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
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
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
    workflowSessionState.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  return {
    ...workflowScreen,
    ...workflowRuntime,
    ...workflowTopology,
    replaceWorkflowDefinitions: workflowSessionState.replaceWorkflowDefinitions,
    upsertWorkflowDefinition: workflowSessionState.upsertWorkflowDefinition,
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
    setWorkflowIntermediateOutputSchema,
    setWorkflowLaunchPolicy,
  }
}

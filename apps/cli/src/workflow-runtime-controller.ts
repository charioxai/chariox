import type {
  QueuedWorkflowLaunch,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowRun,
} from "./cli-types.js"
import {
  cancelWorkflowRunRequest,
  clearQueuedWorkflowLaunchesRequest,
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
  listQueuedWorkflowLaunchesRequest,
  listWorkflowRunsRequest,
  removeQueuedWorkflowLaunchRequest,
  resumeWorkflowRunRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowRuntimeControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowSessionRefresh: (session: RuntimeSession) => void
}

export function createWorkflowRuntimeController(deps: WorkflowRuntimeControllerDeps) {
  const invokeWorkflowEndpoint = async (
    workflowRef: string,
    endpointRef: string,
    prompt?: string | null,
  ) => {
    const response = await deps.sendRequest(
      invokeWorkflowEndpointRequest(deps.sessionId(), workflowRef, endpointRef, prompt),
    )
    if ("WorkflowRunInvoked" in response) {
      const payload = expectVariant<{
        workflow_run: WorkflowRun
        workflow: WorkflowDefinition
        endpoint: WorkflowEndpointDefinition
        session: RuntimeSession
      }>(response, "WorkflowRunInvoked")
      deps.applyWorkflowSessionRefresh(payload.session)
      return payload
    }
    const payload = expectVariant<{
      queued_launch: QueuedWorkflowLaunch
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
      session: RuntimeSession
    }>(response, "WorkflowRunQueued")
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listQueuedWorkflowLaunches = async () => {
    const response = await deps.sendRequest(
      listQueuedWorkflowLaunchesRequest(deps.sessionId()),
    )
    const payload = expectVariant<{ queued_launches: QueuedWorkflowLaunch[] }>(
      response,
      "QueuedWorkflowLaunchesListed",
    )
    return payload.queued_launches
  }

  const removeQueuedWorkflowLaunch = async (queueItemRef: string) => {
    const response = await deps.sendRequest(
      removeQueuedWorkflowLaunchRequest(deps.sessionId(), queueItemRef),
    )
    const payload = expectVariant<{ queued_launch: QueuedWorkflowLaunch; session: RuntimeSession }>(
      response,
      "QueuedWorkflowLaunchRemoved",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const clearQueuedWorkflowLaunches = async () => {
    const response = await deps.sendRequest(
      clearQueuedWorkflowLaunchesRequest(deps.sessionId()),
    )
    const payload = expectVariant<{ queued_launches: QueuedWorkflowLaunch[]; session: RuntimeSession }>(
      response,
      "QueuedWorkflowLaunchesCleared",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listWorkflowRuns = async (workflowRef?: string | null) => {
    const response = await deps.sendRequest(listWorkflowRunsRequest(deps.sessionId(), workflowRef))
    const payload = expectVariant<{ workflow_runs: WorkflowRun[] }>(response, "WorkflowRunsListed")
    return payload.workflow_runs
  }

  const getWorkflowRun = async (workflowRunRef: string) => {
    const response = await deps.sendRequest(getWorkflowRunRequest(deps.sessionId(), workflowRunRef))
    return expectVariant<{ workflow_run: WorkflowRun }>(response, "WorkflowRun")
  }

  const cancelWorkflowRun = async (workflowRunRef: string) => {
    const response = await deps.sendRequest(cancelWorkflowRunRequest(deps.sessionId(), workflowRunRef))
    const payload = expectVariant<{ workflow_run: WorkflowRun; session: RuntimeSession }>(
      response,
      "WorkflowRunCancelled",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const resumeWorkflowRun = async (workflowRunRef: string) => {
    const response = await deps.sendRequest(resumeWorkflowRunRequest(deps.sessionId(), workflowRunRef))
    const payload = expectVariant<{ workflow_run: WorkflowRun; session: RuntimeSession }>(
      response,
      "WorkflowRunResumed",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  return {
    invokeWorkflowEndpoint,
    listQueuedWorkflowLaunches,
    removeQueuedWorkflowLaunch,
    clearQueuedWorkflowLaunches,
    listWorkflowRuns,
    getWorkflowRun,
    cancelWorkflowRun,
    resumeWorkflowRun,
  }
}

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt,
  WorkflowRun,
} from "./cli-types.js"
import {
  cancelWorkflowRunRequest,
  clearWorkflowPromptQueueRequest,
  createWorkflowPromptQueueRequest,
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
  listQueuedWorkflowPromptsRequest,
  listWorkflowPromptQueuesRequest,
  listWorkflowRunsRequest,
  removeQueuedWorkflowPromptRequest,
  removeWorkflowPromptQueueRequest,
  resumeWorkflowRunRequest,
  updateQueuedWorkflowPromptRequest,
  updateWorkflowPromptQueueRequest,
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
    queueRef?: string | null,
  ) => {
    const response = await deps.sendRequest(
      invokeWorkflowEndpointRequest(deps.sessionId(), workflowRef, endpointRef, prompt, queueRef),
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
      queued_prompt: WorkflowQueuedPrompt
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
      session: RuntimeSession
    }>(response, "WorkflowPromptEnqueued")
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listWorkflowPromptQueues = async () => {
    const response = await deps.sendRequest(listWorkflowPromptQueuesRequest(deps.sessionId()))
    const payload = expectVariant<{ queues: WorkflowPromptQueueDefinition[] }>(
      response,
      "WorkflowPromptQueuesListed",
    )
    return payload.queues
  }

  const createWorkflowPromptQueue = async (alias: string, priority: number) => {
    const response = await deps.sendRequest(
      createWorkflowPromptQueueRequest(deps.sessionId(), alias, priority),
    )
    const payload = expectVariant<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>(
      response,
      "WorkflowPromptQueueCreated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const updateWorkflowPromptQueue = async (
    queueRef: string,
    patch: { alias?: string | null; priority?: number | null; enabled?: boolean | null },
  ) => {
    const response = await deps.sendRequest(
      updateWorkflowPromptQueueRequest(deps.sessionId(), queueRef, patch),
    )
    const payload = expectVariant<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>(
      response,
      "WorkflowPromptQueueUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const removeWorkflowPromptQueue = async (queueRef: string) => {
    const response = await deps.sendRequest(removeWorkflowPromptQueueRequest(deps.sessionId(), queueRef))
    const payload = expectVariant<{ queue: WorkflowPromptQueueDefinition; session: RuntimeSession }>(
      response,
      "WorkflowPromptQueueRemoved",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listQueuedWorkflowPrompts = async () => {
    const response = await deps.sendRequest(listQueuedWorkflowPromptsRequest(deps.sessionId()))
    const payload = expectVariant<{ queued_prompts: WorkflowQueuedPrompt[] }>(
      response,
      "QueuedWorkflowPromptsListed",
    )
    return payload.queued_prompts
  }

  const updateQueuedWorkflowPrompt = async (
    queueItemRef: string,
    patch: { prompt?: string | null; queueRef?: string | null },
  ) => {
    const response = await deps.sendRequest(
      updateQueuedWorkflowPromptRequest(deps.sessionId(), queueItemRef, patch),
    )
    const payload = expectVariant<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>(
      response,
      "QueuedWorkflowPromptUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const removeQueuedWorkflowPrompt = async (queueItemRef: string) => {
    const response = await deps.sendRequest(
      removeQueuedWorkflowPromptRequest(deps.sessionId(), queueItemRef),
    )
    const payload = expectVariant<{ queued_prompt: WorkflowQueuedPrompt; session: RuntimeSession }>(
      response,
      "QueuedWorkflowPromptRemoved",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const clearWorkflowPromptQueue = async (queueRef: string) => {
    const response = await deps.sendRequest(clearWorkflowPromptQueueRequest(deps.sessionId(), queueRef))
    const payload = expectVariant<{ queued_prompts: WorkflowQueuedPrompt[]; session: RuntimeSession }>(
      response,
      "WorkflowPromptQueueCleared",
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
    listWorkflowPromptQueues,
    createWorkflowPromptQueue,
    updateWorkflowPromptQueue,
    removeWorkflowPromptQueue,
    listQueuedWorkflowPrompts,
    updateQueuedWorkflowPrompt,
    removeQueuedWorkflowPrompt,
    clearWorkflowPromptQueue,
    listWorkflowRuns,
    getWorkflowRun,
    cancelWorkflowRun,
    resumeWorkflowRun,
  }
}

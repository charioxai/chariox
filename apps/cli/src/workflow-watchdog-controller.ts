import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import {
  createWorkflowWatchdogRequest,
  listWorkflowWatchdogsRequest,
  removeWorkflowWatchdogRequest,
  setWorkflowWatchdogEnabledRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowWatchdogControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowSessionRefresh: (session: RuntimeSession) => void
}

export function createWorkflowWatchdogController(deps: WorkflowWatchdogControllerDeps) {
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
        deps.sessionId(),
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
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listWorkflowWatchdogs = async (workflowRef?: string | null) => {
    const response = await deps.sendRequest(listWorkflowWatchdogsRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ watchdogs: WorkflowWatchdogDefinition[] }>(response, "WorkflowWatchdogsListed")
  }

  const setWorkflowWatchdogEnabled = async (watchdogRef: string, enabled: boolean) => {
    const response = await deps.sendRequest(setWorkflowWatchdogEnabledRequest(deps.sessionId(), watchdogRef, enabled))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const removeWorkflowWatchdog = async (watchdogRef: string) => {
    const response = await deps.sendRequest(removeWorkflowWatchdogRequest(deps.sessionId(), watchdogRef))
    const payload = expectVariant<{ watchdog: WorkflowWatchdogDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogRemoved",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  return {
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
  }
}

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowScheduleDefinition,
  WorkflowScheduleTrigger,
} from "./cli-types.js"
import {
  createWorkflowScheduleRequest,
  createWorkflowWatchdogRequest,
  listWorkflowSchedulesRequest,
  listWorkflowWatchdogsRequest,
  removeWorkflowScheduleRequest,
  removeWorkflowWatchdogRequest,
  setWorkflowScheduleEnabledRequest,
  setWorkflowWatchdogEnabledRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowWatchdogControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowSessionRefresh: (session: RuntimeSession) => void
}

export function createWorkflowWatchdogController(deps: WorkflowWatchdogControllerDeps) {
  const createWorkflowSchedule = async (
    workflowRef: string,
    endpointRef: string,
    trigger: WorkflowScheduleTrigger,
    invocationPrompt: string,
    overlapPolicy: "skip" | "queue",
    maxRuns?: number | null,
    queueRef?: string | null,
  ) => {
    const response = await deps.sendRequest(
      createWorkflowScheduleRequest(
        deps.sessionId(),
        workflowRef,
        endpointRef,
        trigger,
        invocationPrompt,
        overlapPolicy,
        maxRuns,
        queueRef,
      ),
    )
    const payload = expectVariant<{
      schedule: WorkflowScheduleDefinition
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
      session: RuntimeSession
    }>(response, "WorkflowScheduleCreated")
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listWorkflowSchedules = async (workflowRef?: string | null) => {
    const response = await deps.sendRequest(listWorkflowSchedulesRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ schedules: WorkflowScheduleDefinition[] }>(response, "WorkflowSchedulesListed")
  }

  const setWorkflowScheduleEnabled = async (scheduleRef: string, enabled: boolean) => {
    const response = await deps.sendRequest(setWorkflowScheduleEnabledRequest(deps.sessionId(), scheduleRef, enabled))
    const payload = expectVariant<{ schedule: WorkflowScheduleDefinition; session: RuntimeSession }>(
      response,
      "WorkflowScheduleUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const removeWorkflowSchedule = async (scheduleRef: string) => {
    const response = await deps.sendRequest(removeWorkflowScheduleRequest(deps.sessionId(), scheduleRef))
    const payload = expectVariant<{ schedule: WorkflowScheduleDefinition; session: RuntimeSession }>(
      response,
      "WorkflowScheduleRemoved",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
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
      watchdog: WorkflowScheduleDefinition
      workflow: WorkflowDefinition
      endpoint: WorkflowEndpointDefinition
      session: RuntimeSession
    }>(response, "WorkflowWatchdogCreated")
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const listWorkflowWatchdogs = async (workflowRef?: string | null) => {
    const response = await deps.sendRequest(listWorkflowWatchdogsRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ watchdogs: WorkflowScheduleDefinition[] }>(response, "WorkflowWatchdogsListed")
  }

  const setWorkflowWatchdogEnabled = async (watchdogRef: string, enabled: boolean) => {
    const response = await deps.sendRequest(setWorkflowWatchdogEnabledRequest(deps.sessionId(), watchdogRef, enabled))
    const payload = expectVariant<{ watchdog: WorkflowScheduleDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const removeWorkflowWatchdog = async (watchdogRef: string) => {
    const response = await deps.sendRequest(removeWorkflowWatchdogRequest(deps.sessionId(), watchdogRef))
    const payload = expectVariant<{ watchdog: WorkflowScheduleDefinition; session: RuntimeSession }>(
      response,
      "WorkflowWatchdogRemoved",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  return {
    createWorkflowSchedule,
    listWorkflowSchedules,
    setWorkflowScheduleEnabled,
    removeWorkflowSchedule,
    createWorkflowWatchdog,
    listWorkflowWatchdogs,
    setWorkflowWatchdogEnabled,
    removeWorkflowWatchdog,
  }
}

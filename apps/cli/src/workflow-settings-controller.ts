import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import {
  setWorkflowFlushContextRequest,
  setWorkflowRunOutputSchemaRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowSettingsControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowSessionRefresh: (session: RuntimeSession) => void
}

export function createWorkflowSettingsController(deps: WorkflowSettingsControllerDeps) {
  const setWorkflowFlushContext = async (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowFlushContextRequest(
        deps.sessionId(),
        workflowRef,
        flushAgentContextBeforeRun,
      ),
    )
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowFlushContextUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const setWorkflowRunOutputSchema = async (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => {
    const response = await deps.sendRequest(
      setWorkflowRunOutputSchemaRequest(deps.sessionId(), workflowRef, runOutputSchemaRef),
    )
    const payload = expectVariant<{ workflow: WorkflowDefinition; session: RuntimeSession }>(
      response,
      "WorkflowRunOutputSchemaUpdated",
    )
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  return {
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
  }
}

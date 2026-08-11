import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import type { WorkflowDesignOp } from "@arroba/kernel-client/kernel-types"
import { resolveWorkflowRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowSettingsControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowDesignOp: (op: WorkflowDesignOp) => Promise<{ session: RuntimeSession }>
  applyWorkflowSessionRefresh: (session: RuntimeSession) => void
}

export function createWorkflowSettingsController(deps: WorkflowSettingsControllerDeps) {
  const setWorkflowFlushContext = async (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    const accepted = await deps.applyWorkflowDesignOp({
      kind: "workflow_update",
      workflow_id: workflow.id,
      patch: { flush_agent_context_before_run: flushAgentContextBeforeRun },
    })
    const payload = { ...accepted, workflow: workflowFromSession(accepted.session, workflow.id) }
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  const setWorkflowRunOutputSchema = async (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => {
    const workflow = await resolveWorkflow(workflowRef)
    const accepted = await deps.applyWorkflowDesignOp({
      kind: "workflow_update",
      workflow_id: workflow.id,
      patch: { run_output_schema_ref: runOutputSchemaRef },
    })
    const payload = { ...accepted, workflow: workflowFromSession(accepted.session, workflow.id) }
    deps.applyWorkflowSessionRefresh(payload.session)
    return payload
  }

  return {
    setWorkflowFlushContext,
    setWorkflowRunOutputSchema,
  }

  async function resolveWorkflow(workflowRef: string): Promise<WorkflowDefinition> {
    const response = await deps.sendRequest(resolveWorkflowRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved").workflow
  }
}

function workflowFromSession(session: RuntimeSession, workflowId: string): WorkflowDefinition {
  const workflow = session.workflows?.find((candidate) => candidate.id === workflowId)
  if (!workflow) throw new Error(`workflow design response did not include workflow ${workflowId}`)
  return workflow
}

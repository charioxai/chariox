import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import type { WorkflowDesignOp } from "@chariox/kernel-client/kernel-types"
import {
  listWorkflowsRequest,
  resolveWorkflowRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowDefinitionControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applyWorkflowDesignOp: (op: WorkflowDesignOp) => Promise<{ session: RuntimeSession }>
  createWorkflowDesignId: (prefix: string) => string
  applySessionState: (session: RuntimeSession) => void
  setSelectedWorkflowId: (value: string | null) => void
  setSelectedWorkflowNodeId: (value: string | null) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
}

export function createWorkflowDefinitionController(deps: WorkflowDefinitionControllerDeps) {
  const createWorkflow = async (alias?: string | null) => {
    const workflowId = deps.createWorkflowDesignId("workflow")
    const accepted = await deps.applyWorkflowDesignOp({
      kind: "workflow_create",
      workflow: { id: workflowId, alias: alias ?? null },
    })
    const workflow = workflowFromSession(accepted.session, workflowId)
    const payload = { ...accepted, workflow }
    deps.applySessionState(payload.session)
    deps.setSelectedWorkflowId(payload.workflow.id)
    deps.setSelectedWorkflowNodeId(null)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
    return payload
  }

  const listWorkflows = async () => {
    const response = await deps.sendRequest(listWorkflowsRequest(deps.sessionId()))
    const payload = expectVariant<{ workflows: WorkflowDefinition[] }>(response, "WorkflowsListed")
    return payload.workflows
  }

  const resolveWorkflow = async (workflowRef: string) => {
    const response = await deps.sendRequest(resolveWorkflowRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved")
  }

  const assignWorkflowAlias = async (workflowId: string, alias: string) => {
    const { workflow } = await resolveWorkflow(workflowId)
    const accepted = await deps.applyWorkflowDesignOp({
      kind: "workflow_update",
      workflow_id: workflow.id,
      patch: { alias },
    })
    const updatedWorkflow = workflowFromSession(accepted.session, workflow.id)
    const payload = { ...accepted, workflow: updatedWorkflow }
    deps.applySessionState(payload.session)
    if (payload.workflow) {
      deps.rebuildTranscript()
      deps.applyResponseLayout()
    }
    return payload.workflow
  }

  const deleteWorkflow = async (workflowRef: string) => {
    const { workflow } = await resolveWorkflow(workflowRef)
    const payload = await deps.applyWorkflowDesignOp({
      kind: "workflow_remove",
      workflow_id: workflow.id,
    })
    return { workflow, session: payload.session }
  }

  return {
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
    deleteWorkflow,
  }
}

function workflowFromSession(session: RuntimeSession, workflowId: string): WorkflowDefinition {
  const workflow = session.workflows?.find((candidate) => candidate.id === workflowId)
  if (!workflow) throw new Error(`workflow design response did not include workflow ${workflowId}`)
  return workflow
}

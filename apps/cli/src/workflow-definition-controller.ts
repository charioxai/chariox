import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import {
  aliasWorkflowRequest,
  createWorkflowRequest,
  listWorkflowsRequest,
  resolveWorkflowRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

type WorkflowDefinitionControllerDeps = {
  sendRequest: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  sessionId: () => string
  applySessionState: (session: RuntimeSession) => void
  setSelectedWorkflowId: (value: string | null) => void
  setSelectedWorkflowNodeId: (value: string | null) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
}

export function createWorkflowDefinitionController(deps: WorkflowDefinitionControllerDeps) {
  const createWorkflow = async (alias?: string | null) => {
    const response = await deps.sendRequest(createWorkflowRequest(deps.sessionId(), alias))
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
    const response = await deps.sendRequest(listWorkflowsRequest(deps.sessionId()))
    const payload = expectVariant<{ workflows: WorkflowDefinition[] }>(response, "WorkflowsListed")
    return payload.workflows
  }

  const resolveWorkflow = async (workflowRef: string) => {
    const response = await deps.sendRequest(resolveWorkflowRequest(deps.sessionId(), workflowRef))
    return expectVariant<{ workflow: WorkflowDefinition }>(response, "WorkflowResolved")
  }

  const assignWorkflowAlias = async (workflowId: string, alias: string) => {
    const response = await deps.sendRequest(aliasWorkflowRequest(deps.sessionId(), workflowId, alias))
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

  return {
    createWorkflow,
    listWorkflows,
    resolveWorkflow,
    assignWorkflowAlias,
  }
}

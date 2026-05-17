import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"

type WorkflowSessionStateControllerDeps = {
  sessionState: () => RuntimeSession
  applySessionState: (session: RuntimeSession) => void
  rebuildTranscript: () => void
  applyResponseLayout: () => void
}

export function createWorkflowSessionStateController(
  deps: WorkflowSessionStateControllerDeps,
) {
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

  const applyWorkflowSessionRefresh = (session: RuntimeSession) => {
    deps.applySessionState(session)
    deps.rebuildTranscript()
    deps.applyResponseLayout()
  }

  return {
    replaceWorkflowDefinitions,
    upsertWorkflowDefinition,
    applyWorkflowSessionRefresh,
  }
}

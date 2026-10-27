export type AgentPaneRuntimeResetControllerDeps = {
  clearRenderedPanes: () => void
  clearCurrentAuxiliaryAgentIds: () => void
}

export function createAgentPaneRuntimeResetController(
  deps: AgentPaneRuntimeResetControllerDeps,
) {
  const reset = () => {
    deps.clearRenderedPanes()
    deps.clearCurrentAuxiliaryAgentIds()
  }

  return {
    reset,
  }
}

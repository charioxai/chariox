export type CliLoadingStateControllerDeps = {
  getSessionHydrating: () => boolean
  setSessionHydrating: (next: boolean) => void
  setLoadingHistory: (next: boolean) => void
  renderHistoryLoadingIndicator: () => void
  isAttached: () => boolean
  visibleTranscriptEntryCount: () => number
  workflowScreenActive: () => boolean
  rebuildTranscript: () => void
  requestTranscriptRender: () => void
}

export function createCliLoadingStateController(
  deps: CliLoadingStateControllerDeps,
) {
  const setHistoryLoadingState = (next: boolean) => {
    deps.setLoadingHistory(next)
    deps.renderHistoryLoadingIndicator()
  }

  const setSessionHydratingState = (next: boolean) => {
    if (deps.getSessionHydrating() === next) {
      return false
    }
    deps.setSessionHydrating(next)
    if (
      deps.isAttached()
      && deps.visibleTranscriptEntryCount() === 0
      && !deps.workflowScreenActive()
    ) {
      deps.rebuildTranscript()
      return true
    }
    deps.requestTranscriptRender()
    return true
  }

  return {
    setHistoryLoadingState,
    setSessionHydratingState,
  }
}

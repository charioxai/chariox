export type VisibleActivityLabelControllerDeps = {
  focusedActivityLabel: () => string | null
  setActiveStatusLabel: (label: string | null) => void
}

export function createVisibleActivityLabelController(
  deps: VisibleActivityLabelControllerDeps,
) {
  return {
    sync() {
      deps.setActiveStatusLabel(deps.focusedActivityLabel())
    },
  }
}

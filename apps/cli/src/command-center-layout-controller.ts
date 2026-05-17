export type CommandCenterLayoutControllerDeps = {
  terminalHeight: () => number
  promptHeight: () => number
}

export function createCommandCenterLayoutController(
  deps: CommandCenterLayoutControllerDeps,
) {
  return {
    visibleRowCount() {
      return Math.max(4, Math.min(10, deps.terminalHeight() - deps.promptHeight() - 10))
    },
  }
}

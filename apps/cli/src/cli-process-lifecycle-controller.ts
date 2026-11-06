export type CliProcessLifecycleControllerDeps = {
  handleSigint: () => void
  startAutomationServer: () => void
  stopAutomationServer: () => void
  onSigint: (handler: () => void) => void
  offSigint: (handler: () => void) => void
  clearTerminalOutputRecordTimer: () => void
}

export function createCliProcessLifecycleController(
  deps: CliProcessLifecycleControllerDeps,
) {
  let started = false

  return {
    start() {
      if (started) {
        return
      }
      started = true
      deps.startAutomationServer()
      deps.onSigint(deps.handleSigint)
    },
    stop() {
      if (!started) {
        return
      }
      started = false
      deps.offSigint(deps.handleSigint)
      deps.stopAutomationServer()
      deps.clearTerminalOutputRecordTimer()
    },
  }
}

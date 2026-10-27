export type CliProcessLifecycleControllerDeps = {
  handleSigint: () => void
  handleStdinData: (chunk: Buffer | string) => void
  startAutomationServer: () => void
  stopAutomationServer: () => void
  onSigint: (handler: () => void) => void
  offSigint: (handler: () => void) => void
  onStdinData: (handler: (chunk: Buffer | string) => void) => void
  offStdinData: (handler: (chunk: Buffer | string) => void) => void
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
      deps.onStdinData(deps.handleStdinData)
    },
    stop() {
      if (!started) {
        return
      }
      started = false
      deps.offSigint(deps.handleSigint)
      deps.offStdinData(deps.handleStdinData)
      deps.stopAutomationServer()
      deps.clearTerminalOutputRecordTimer()
    },
  }
}

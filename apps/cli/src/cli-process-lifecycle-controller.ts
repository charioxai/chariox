// MP-08/MP-11: Chariox owns SIGINT until draft/archive/detach cleanup finishes.
// Preserve OpenTUI 0.1.87's other terminal restoration signals.
export const CLI_RENDERER_EXIT_SIGNALS: NodeJS.Signals[] = [
  "SIGTERM", "SIGQUIT", "SIGABRT", "SIGHUP", "SIGBREAK", "SIGPIPE", "SIGBUS", "SIGFPE",
]

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

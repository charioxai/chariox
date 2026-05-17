export type HotkeyDebugReporterDeps = {
  debugLogsEnabled: boolean
  logDebug: (message: string, fields?: Record<string, unknown>) => void
  flashFooter: (message: string, tone: "info") => void
}

export function createHotkeyDebugReporter(deps: HotkeyDebugReporterDeps) {
  return {
    report(message: string) {
      deps.logDebug("hotkeys footer debug", { detail: message })
      if (!deps.debugLogsEnabled) {
        return
      }
      deps.flashFooter(`[hotkeys] ${message}`, "info")
    },
  }
}

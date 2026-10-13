export type DaemonActivityControllerDeps = {
  recordConnectionActivity: () => void
  daemonDisconnected: () => boolean
  setDaemonDisconnected: (disconnected: boolean) => void
  updateSessionChrome: () => void
  onRecovered?: () => void
}

export function createDaemonActivityController(deps: DaemonActivityControllerDeps) {
  return {
    record(_activityType: string) {
      deps.recordConnectionActivity()
      if (!deps.daemonDisconnected()) {
        return
      }
      deps.setDaemonDisconnected(false)
      deps.onRecovered?.()
      deps.updateSessionChrome()
    },
  }
}

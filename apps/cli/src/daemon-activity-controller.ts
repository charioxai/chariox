export type DaemonActivityControllerDeps = {
  recordConnectionActivity: () => void
  daemonDisconnected: () => boolean
  setDaemonDisconnected: (disconnected: boolean) => void
  updateSessionChrome: () => void
  onRecovered?: () => void
  canRecoverConnection?: () => boolean
}

export function createDaemonActivityController(deps: DaemonActivityControllerDeps) {
  return {
    record(_activityType: string) {
      deps.recordConnectionActivity()
      if (!deps.daemonDisconnected() || deps.canRecoverConnection?.() === false) {
        return
      }
      deps.setDaemonDisconnected(false)
      deps.onRecovered?.()
      deps.updateSessionChrome()
    },
  }
}

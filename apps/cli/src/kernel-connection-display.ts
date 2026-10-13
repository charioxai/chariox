// MP-08/MP-10: connection health belongs in shared chrome, never in agent output.
export type KernelConnectionDisplayTimer = {
  setTimeout(callback: () => void, delayMs: number): ReturnType<typeof setTimeout>
  clearTimeout(timer: ReturnType<typeof setTimeout>): void
}

export function createKernelConnectionDisplay(deps: {
  setStatusLine(message: string): void
  updateSessionChrome(): void
  connectedStatusLine: string
  daemonDisconnected?: () => boolean
  setDaemonDisconnected?: (value: boolean) => void
  timer?: KernelConnectionDisplayTimer
}) {
  const timer = deps.timer ?? { setTimeout, clearTimeout }
  let disconnected = false
  let deadline: ReturnType<typeof setTimeout> | null = null
  const clear = () => {
    if (deadline !== null) timer.clearTimeout(deadline)
    deadline = null
  }
  return {
    closed() {
      // A switch/attach may restore the shared signal without a resumed event.
      if (deps.daemonDisconnected?.() === false) {
        clear()
        disconnected = false
      }
      deps.setDaemonDisconnected?.(true)
      if (disconnected) return
      disconnected = true
      deps.setStatusLine("Reconnecting to the Chariox kernel…")
      deps.updateSessionChrome()
      deadline = timer.setTimeout(() => {
        deadline = null
        if (!disconnected || deps.daemonDisconnected?.() === false) {
          disconnected = false
          return
        }
        deps.setStatusLine("Disconnected from the Chariox kernel · showing last known data")
        deps.updateSessionChrome()
      }, 10_000)
      deadline.unref?.()
    },
    resumed() {
      clear()
      disconnected = false
      deps.setStatusLine(deps.connectedStatusLine)
      deps.updateSessionChrome()
    },
    dispose() { clear(); disconnected = false },
  }
}

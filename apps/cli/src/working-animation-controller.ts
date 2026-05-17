export type WorkingAnimationControllerDeps<TimerHandle> = {
  intervalMs: number
  scheduleInterval: (callback: () => void, intervalMs: number) => TimerHandle
  clearInterval: (handle: TimerHandle) => void
  incrementFrame: () => void
  sessionStatusMode: () => string
  splitAgentResponseMode: () => boolean
  updateSessionChrome: () => void
  renderSplitPaneFooters: () => void
}

export type WorkingAnimationController = {
  start(): void
  stop(): void
  tick(): void
}

export function createWorkingAnimationController<TimerHandle>(
  deps: WorkingAnimationControllerDeps<TimerHandle>,
): WorkingAnimationController {
  let timer: TimerHandle | null = null

  const tick = () => {
    deps.incrementFrame()
    if (deps.sessionStatusMode() === "working") {
      deps.updateSessionChrome()
    }
    if (deps.splitAgentResponseMode()) {
      deps.renderSplitPaneFooters()
    }
  }

  return {
    start() {
      if (timer !== null) {
        return
      }
      timer = deps.scheduleInterval(tick, deps.intervalMs)
    },
    stop() {
      if (timer === null) {
        return
      }
      deps.clearInterval(timer)
      timer = null
    },
    tick,
  }
}

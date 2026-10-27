export type WorkingAnimationControllerDeps<TimerHandle> = {
  intervalMs: number
  scheduleInterval: (callback: () => void, intervalMs: number) => TimerHandle
  clearInterval: (handle: TimerHandle) => void
  incrementFrame: () => void
  sessionStatusMode: () => string
  hasWaitingDeadline?: () => boolean
  hasGoalClock?: () => boolean
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

  let deadlineElapsedMs = 0
  const tick = () => {
    deadlineElapsedMs += deps.intervalMs
    const countdownDue = Boolean(deps.hasWaitingDeadline?.()) && deadlineElapsedMs >= 1000
    if (countdownDue) deadlineElapsedMs = 0
    deps.incrementFrame()
    if (deps.sessionStatusMode() === "working" || countdownDue || deps.hasGoalClock?.()) {
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

export type TranscriptScrollMonitorControllerDeps<TimerHandle> = {
  intervalMs: number
  scheduleInterval: (callback: () => void, intervalMs: number) => TimerHandle
  clearInterval: (handle: TimerHandle) => void
  monitorScroll: () => void
}

export type TranscriptScrollMonitorController = {
  start(): void
  stop(): void
  tick(): void
}

export function createTranscriptScrollMonitorController<TimerHandle>(
  deps: TranscriptScrollMonitorControllerDeps<TimerHandle>,
): TranscriptScrollMonitorController {
  let timer: TimerHandle | null = null

  const tick = () => {
    deps.monitorScroll()
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

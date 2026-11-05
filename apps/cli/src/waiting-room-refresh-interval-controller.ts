export type WaitingRoomRefreshIntervalControllerDeps<TimerHandle> = {
  intervalMs: number
  scheduleInterval: (callback: () => void, intervalMs: number) => TimerHandle
  clearInterval: (handle: TimerHandle) => void
  refreshWaitingRoomData: () => Promise<void> | void
}

export type WaitingRoomRefreshIntervalController = {
  start(): void
  stop(): void
  tick(): void
}

export function createWaitingRoomRefreshIntervalController<TimerHandle>(
  deps: WaitingRoomRefreshIntervalControllerDeps<TimerHandle>,
): WaitingRoomRefreshIntervalController {
  let timer: TimerHandle | null = null

  const tick = () => {
    void deps.refreshWaitingRoomData()
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

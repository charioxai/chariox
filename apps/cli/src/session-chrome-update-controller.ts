type SessionChromeUpdateControllerOptions<TimerHandle> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  isBatched: () => boolean
  applyUpdate: () => void
}

export type SessionChromeUpdateController = {
  request(throttle: boolean): void
  flush(): void
  flushDeferred(): void
  clearTimer(): void
}

export function createSessionChromeUpdateController<TimerHandle>(
  options: SessionChromeUpdateControllerOptions<TimerHandle>,
): SessionChromeUpdateController {
  let pendingTimer: TimerHandle | undefined
  let deferredUpdate = false

  const clearPendingTimer = () => {
    if (pendingTimer === undefined) {
      return
    }
    options.clearTimer(pendingTimer)
    pendingTimer = undefined
  }

  const applyOrDefer = () => {
    if (options.isBatched()) {
      deferredUpdate = true
      return
    }
    options.applyUpdate()
  }

  const flush = () => {
    clearPendingTimer()
    applyOrDefer()
  }

  return {
    request(throttle) {
      if (options.isBatched()) {
        deferredUpdate = true
        return
      }
      if (!throttle) {
        flush()
        return
      }
      if (pendingTimer !== undefined) {
        return
      }
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        applyOrDefer()
      }, options.delayMs)
    },
    flush,
    flushDeferred() {
      if (!deferredUpdate) {
        return
      }
      deferredUpdate = false
      flush()
    },
    clearTimer() {
      clearPendingTimer()
    },
  }
}

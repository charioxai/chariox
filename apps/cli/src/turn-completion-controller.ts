type TurnCompletionControllerOptions<TimerHandle> = {
  now: () => number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  hasActivePrompt: () => boolean
  getDelayMs: (lastActivityAt: number) => number | null
  completeTurn: () => void
}

export type TurnCompletionController = {
  recordActivity(): void
  reset(): void
  confirm(): void
  confirmAndSchedule(): void
  maybeScheduleConfirmed(): void
  handleProviderActivity(active: boolean): void
  cancelPending(): void
  isConfirmed(): boolean
}

export function createTurnCompletionController<TimerHandle>(
  options: TurnCompletionControllerOptions<TimerHandle>,
): TurnCompletionController {
  let pendingTimer: TimerHandle | undefined
  let confirmed = false
  let lastActivityAt = options.now()

  const cancelPending = () => {
    if (pendingTimer === undefined) {
      return
    }
    options.clearTimer(pendingTimer)
    pendingTimer = undefined
  }

  const scheduleCompletion = () => {
    cancelPending()
    const delayMs = options.getDelayMs(lastActivityAt)
    if (delayMs === null) {
      return
    }
    pendingTimer = options.scheduleTimer(() => {
      pendingTimer = undefined
      finalizeCompletion()
    }, delayMs)
  }

  const finalizeCompletion = () => {
    cancelPending()
    const delayMs = options.getDelayMs(lastActivityAt)
    if (delayMs === null) {
      return
    }
    if (delayMs > 0) {
      scheduleCompletion()
      return
    }
    options.completeTurn()
    confirmed = false
  }

  const maybeScheduleConfirmed = () => {
    if (!confirmed || options.hasActivePrompt()) {
      return
    }
    scheduleCompletion()
  }

  return {
    recordActivity() {
      lastActivityAt = options.now()
      cancelPending()
    },
    reset() {
      confirmed = false
      cancelPending()
    },
    confirm() {
      confirmed = true
    },
    confirmAndSchedule() {
      confirmed = true
      maybeScheduleConfirmed()
    },
    maybeScheduleConfirmed,
    handleProviderActivity(active) {
      if (active) {
        cancelPending()
        return
      }
      maybeScheduleConfirmed()
    },
    cancelPending,
    isConfirmed() {
      return confirmed
    },
  }
}

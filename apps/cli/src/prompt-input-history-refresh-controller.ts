type PromptInputHistoryRefreshControllerOptions<TimerHandle> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  refreshHistory: (sessionId: string) => Promise<void>
  onRefreshError?: (error: unknown, sessionId: string) => void
}

export type PromptInputHistoryRefreshController = {
  refresh(sessionId: string): Promise<void>
  schedule(sessionId: string): void
  clearTimer(): void
}

export function createPromptInputHistoryRefreshController<TimerHandle>(
  options: PromptInputHistoryRefreshControllerOptions<TimerHandle>,
): PromptInputHistoryRefreshController {
  let refreshInFlight: Promise<void> | null = null
  let pendingTimer: TimerHandle | undefined

  const clearPendingTimer = () => {
    if (pendingTimer === undefined) {
      return
    }
    options.clearTimer(pendingTimer)
    pendingTimer = undefined
  }

  const refresh = (sessionId: string) => {
    if (refreshInFlight) {
      return refreshInFlight
    }
    refreshInFlight = options.refreshHistory(sessionId)
      .finally(() => {
        refreshInFlight = null
      })
    return refreshInFlight
  }

  return {
    refresh,
    schedule(sessionId) {
      if (pendingTimer !== undefined) {
        return
      }
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        void refresh(sessionId).catch((error) => {
          options.onRefreshError?.(error, sessionId)
        })
      }, options.delayMs)
    },
    clearTimer() {
      clearPendingTimer()
    },
  }
}

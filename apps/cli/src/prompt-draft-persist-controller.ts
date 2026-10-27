export type PromptDraftPersistRequest = {
  sessionId: string
  promptDraft: string
}

type PromptDraftPersistControllerOptions<TimerHandle> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  persistPromptDraft: (request: PromptDraftPersistRequest) => Promise<void>
  onPersistError?: (error: unknown, request: PromptDraftPersistRequest) => void
}

export type PromptDraftPersistController = {
  schedule(sessionId: string, promptDraft: string): void
  flush(): Promise<void>
  clearTimer(): void
  clearPending(): void
}

export function createPromptDraftPersistController<TimerHandle>(
  options: PromptDraftPersistControllerOptions<TimerHandle>,
): PromptDraftPersistController {
  let pendingTimer: TimerHandle | undefined
  let pendingRequest: PromptDraftPersistRequest | null = null

  const clearPendingTimer = () => {
    if (pendingTimer === undefined) {
      return
    }
    options.clearTimer(pendingTimer)
    pendingTimer = undefined
  }

  const takePendingRequest = () => {
    const request = pendingRequest
    pendingRequest = null
    return request
  }

  const persist = async (request: PromptDraftPersistRequest) => {
    await options.persistPromptDraft(request)
  }

  return {
    schedule(sessionId, promptDraft) {
      pendingRequest = { sessionId, promptDraft }
      clearPendingTimer()
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        const request = takePendingRequest()
        if (!request) {
          return
        }
        void persist(request).catch((error) => {
          options.onPersistError?.(error, request)
        })
      }, options.delayMs)
    },
    async flush() {
      clearPendingTimer()
      const request = takePendingRequest()
      if (!request) {
        return
      }
      await persist(request)
    },
    clearTimer() {
      clearPendingTimer()
    },
    clearPending() {
      pendingRequest = null
      clearPendingTimer()
    },
  }
}

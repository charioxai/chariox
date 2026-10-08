type KernelEventSubscriptionAttachment = {
  id: string
}

type KernelEventSubscriptionScope = "session" | "waiting-room" | null

type KernelEventSubscriptionState = {
  scope: KernelEventSubscriptionScope
  sessionId: string | null
  attachmentId: string | null
}

type KernelEventSubscriptionControllerOptions = {
  supportsKernelEventStream: () => boolean
  getAttachment: () => KernelEventSubscriptionAttachment | null
  getSessionId: () => string
  subscribeToWaitingRoomInventory: () => Promise<void>
  subscribeToKernelEvents: (sessionId: string, attachmentId: string) => Promise<void>
  onEvaluate: (state: KernelEventSubscriptionState & {
    nextSessionId: string | null
    nextAttachmentId: string | null
    attached: boolean
  }) => void
  onWaitingRoomSubscribed: () => void
  onSessionSubscribed: (sessionId: string, attachmentId: string) => void
  onWaitingRoomSubscriptionFailed: (error: unknown) => void
  onSessionSubscriptionFailed: (sessionId: string, attachmentId: string, error: unknown) => void
}

export type KernelEventSubscriptionController = {
  sync(): Promise<void>
  reset(): void
  state(): KernelEventSubscriptionState
}

export function createKernelEventSubscriptionController(
  options: KernelEventSubscriptionControllerOptions,
): KernelEventSubscriptionController {
  let scope: KernelEventSubscriptionScope = null
  let sessionId: string | null = null
  let attachmentId: string | null = null

  const state = (): KernelEventSubscriptionState => ({
    scope,
    sessionId,
    attachmentId,
  })

  // MP-08: overlapping syncs must not race two subscribes for one stream;
  // a sync requested while one runs re-evaluates once afterwards.
  let running: Promise<void> | null = null
  let rerun = false
  const sync = (): Promise<void> => {
    if (running) {
      rerun = true
      return running
    }
    running = (async () => {
      // Cleared in the same step as the last rerun check, so no later sync
      // can join a run that will not look at it.
      try {
        do {
          rerun = false
          await syncOnce()
        } while (rerun)
      } finally {
        running = null
      }
    })()
    return running
  }

  const syncOnce = async (): Promise<void> => {
    if (!options.supportsKernelEventStream()) {
      return
    }

    const attachment = options.getAttachment()
    const nextSessionId = attachment ? options.getSessionId() : null
    const nextAttachmentId = attachment?.id ?? null
    options.onEvaluate({
      ...state(),
      nextSessionId,
      nextAttachmentId,
      attached: Boolean(attachment),
    })

    if (!attachment || !nextSessionId) {
      if (scope === "waiting-room") {
        return
      }
      try {
        await options.subscribeToWaitingRoomInventory()
        scope = "waiting-room"
        attachmentId = null
        sessionId = null
        options.onWaitingRoomSubscribed()
      } catch (error) {
        options.onWaitingRoomSubscriptionFailed(error)
      }
      return
    }

    if (scope === "session" && attachmentId === attachment.id && sessionId === nextSessionId) {
      return
    }

    try {
      await options.subscribeToKernelEvents(nextSessionId, attachment.id)
      scope = "session"
      attachmentId = attachment.id
      sessionId = nextSessionId
      options.onSessionSubscribed(nextSessionId, attachment.id)
    } catch (error) {
      options.onSessionSubscriptionFailed(nextSessionId, attachment.id, error)
    }
  }

  return {
    sync,
    reset() {
      scope = null
      attachmentId = null
      sessionId = null
    },
    state,
  }
}

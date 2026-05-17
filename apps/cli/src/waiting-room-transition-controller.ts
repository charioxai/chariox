type WaitingRoomTransitionAttachment = {
  id: string
  session_id?: string | null
}

type WaitingRoomTransitionControllerOptions = {
  isClosing: () => boolean
  getCreatedSession: () => boolean
  getConnectedClientCount: () => number
  getAttachment: () => WaitingRoomTransitionAttachment | null
  getSessionId: () => string
  getPromptDraft: () => string
  syncPromptTextSnapshot: () => void
  flushPromptDraftPersist: () => Promise<void>
  persistSessionPromptDraft: (sessionId: string, promptDraft: string) => Promise<void>
  shouldEndSessionOnExit: (createdSession: boolean, connectedClientCount: number) => boolean
  archiveSession: (sessionId: string) => Promise<void>
  detachAttachment: (attachmentId: string) => Promise<void>
  transitionToWaitingRoom: (message: string) => void
  onWaitingRoomRequested: (createdSession: boolean) => void
  onPromptDraftFlushFailed: (error: unknown) => void
  onPromptDraftPersistFailed: (sessionId: string, error: unknown) => void
  onCleanupFailed: (error: unknown) => void
  onTransitionCompleted: () => void
}

export type WaitingRoomTransitionController = {
  requestWaitingRoom(): Promise<boolean>
}

export function createWaitingRoomTransitionController(
  options: WaitingRoomTransitionControllerOptions,
): WaitingRoomTransitionController {
  return {
    async requestWaitingRoom() {
      if (options.isClosing()) {
        return false
      }

      options.onWaitingRoomRequested(options.getCreatedSession())
      try {
        options.syncPromptTextSnapshot()
        await options.flushPromptDraftPersist().catch((error) => {
          options.onPromptDraftFlushFailed(error)
        })
        const sessionId = options.getAttachment()?.session_id
        if (sessionId) {
          await options.persistSessionPromptDraft(sessionId, options.getPromptDraft()).catch((error) => {
            options.onPromptDraftPersistFailed(sessionId, error)
          })
        }

        const attachment = options.getAttachment()
        if (attachment && options.shouldEndSessionOnExit(options.getCreatedSession(), options.getConnectedClientCount())) {
          await options.archiveSession(options.getSessionId())
        } else if (attachment) {
          await options.detachAttachment(attachment.id)
        }
      } catch (error) {
        options.onCleanupFailed(error)
      }

      options.transitionToWaitingRoom("Returned to waiting room.")
      options.onTransitionCompleted()
      return true
    },
  }
}

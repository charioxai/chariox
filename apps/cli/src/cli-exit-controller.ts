type CliExitAttachment = {
  id: string
  session_id?: string | null
}

type CliExitCleanupDecision = {
  exit: boolean
  exitCode: number
  message: string
}

type CliExitControllerOptions = {
  isClosing: () => boolean
  setClosing: (closing: boolean) => void
  getCreatedSession: () => boolean
  getConnectedClientCount: () => number
  getAttachment: () => CliExitAttachment | null
  getSessionId: () => string
  getPromptDraft: () => string
  syncPromptTextSnapshot: () => void
  flushPromptDraftPersist: () => Promise<void>
  persistSessionPromptDraft: (sessionId: string, promptDraft: string) => Promise<void>
  shouldEndSessionOnExit: (createdSession: boolean, connectedClientCount: number) => boolean
  archiveSession: (sessionId: string) => Promise<void>
  detachAttachment: (attachmentId: string) => Promise<void>
  getCleanupDecision: (error: unknown, previousCleanupFailure: boolean) => CliExitCleanupDecision
  restoreTerminalAndExit: (exitCode: number) => Promise<void>
  onForceExitAfterCleanupFailure: () => void
  onExitRequested: (createdSession: boolean) => void
  onPromptDraftFlushFailed: (error: unknown) => void
  onPromptDraftPersistFailed: (sessionId: string, error: unknown) => void
  onCleanupFailed: (decision: CliExitCleanupDecision, error: unknown) => void
  onCleanupCompleted: () => void
}

export type CliExitController = {
  requestExit(): Promise<boolean>
  cleanupFailed(): boolean
}

export function createCliExitController(options: CliExitControllerOptions): CliExitController {
  let cleanupFailed = false

  return {
    async requestExit() {
      if (options.isClosing() && cleanupFailed) {
        options.onForceExitAfterCleanupFailure()
        await options.restoreTerminalAndExit(1)
        return true
      }
      if (options.isClosing()) {
        return false
      }

      options.setClosing(true)
      options.onExitRequested(options.getCreatedSession())
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
        cleanupFailed = false
      } catch (error) {
        const decision = options.getCleanupDecision(error, cleanupFailed)
        cleanupFailed = true
        options.setClosing(false)
        options.onCleanupFailed(decision, error)
        if (decision.exit) {
          await options.restoreTerminalAndExit(decision.exitCode)
        }
        return true
      }

      options.onCleanupCompleted()
      await options.restoreTerminalAndExit(0)
      return true
    },
    cleanupFailed() {
      return cleanupFailed
    },
  }
}

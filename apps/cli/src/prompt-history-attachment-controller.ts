export type PromptHistoryAttachmentControllerDeps = {
  getAttachedSessionId: () => string | null
  restorePromptHistory: (sessionId: string | null) => void
  invalidateHydration: () => void
  hydratePromptHistory: (sessionId: string) => Promise<void>
  isCurrentSession: (sessionId: string) => boolean
  warnHydrationError?: (sessionId: string, error: unknown) => void
}

export type PromptHistoryAttachmentController = {
  sync(): Promise<void> | null
}

export function createPromptHistoryAttachmentController(
  deps: PromptHistoryAttachmentControllerDeps,
): PromptHistoryAttachmentController {
  let hydratedSessionId: string | null | undefined

  return {
    sync() {
      const attachedSessionId = deps.getAttachedSessionId()
      if (attachedSessionId === hydratedSessionId) {
        return null
      }
      hydratedSessionId = attachedSessionId
      deps.restorePromptHistory(attachedSessionId)
      if (!attachedSessionId) {
        deps.invalidateHydration()
        return null
      }
      return deps.hydratePromptHistory(attachedSessionId).catch((error) => {
        if (!deps.isCurrentSession(attachedSessionId)) {
          return
        }
        deps.warnHydrationError?.(attachedSessionId, error)
      })
    },
  }
}

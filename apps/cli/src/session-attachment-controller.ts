import type { RuntimeAttachment, RuntimeSession } from "./cli-types.js"

type AttachedBinding = {
  sessionId: string
  attachmentId: string
}

type SessionAttachmentControllerDeps = {
  isAttached: () => boolean
  attachmentState: () => RuntimeAttachment | null
  sessionState: () => RuntimeSession
  getSessionState: (sessionId: string) => Promise<RuntimeSession>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  refreshSplitPaneFocusRepaint?: () => void
  maybeResize?: (sessionId: string) => Promise<void>
  catchUpAttachedSession?: (
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
  ) => Promise<void>
  formatError?: (error: unknown) => string
  logWarning?: (message: string, fields?: Record<string, unknown>) => void
}

type FinalizeAttachedSessionOptions = {
  sessionId: string
  attachmentId: string
  session: RuntimeSession
}

export function createSessionAttachmentController(deps: SessionAttachmentControllerDeps) {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const currentBinding = (): AttachedBinding | null => {
    if (!deps.isAttached()) {
      return null
    }
    const attachment = deps.attachmentState()
    if (!attachment) {
      return null
    }
    return {
      sessionId: deps.sessionState().id,
      attachmentId: attachment.id,
    }
  }

  const bindingStillCurrent = (binding: AttachedBinding) => {
    const attachment = deps.attachmentState()
    return attachment?.id === binding.attachmentId && deps.sessionState().id === binding.sessionId
  }

  const hydrateCurrentAttachedSession = async (reason = "mount"): Promise<RuntimeSession | null> => {
    const binding = currentBinding()
    if (!binding) {
      return null
    }

    try {
      const latestSession = await deps.getSessionState(binding.sessionId)
      if (!bindingStillCurrent(binding)) {
        return null
      }
      deps.applySessionState(latestSession)
      await deps.refreshAgentPanes(latestSession)
      deps.refreshSplitPaneFocusRepaint?.()
      return latestSession
    } catch (error) {
      deps.logWarning?.("failed to hydrate attached session state", {
        session_id: binding.sessionId,
        attachment_id: binding.attachmentId,
        reason,
        error: formatError(error),
      })
      if (bindingStillCurrent(binding)) {
        await deps.refreshAgentPanes(deps.sessionState())
        deps.refreshSplitPaneFocusRepaint?.()
      }
      return null
    }
  }

  const finalizeAttachedSessionBinding = async (
    options: FinalizeAttachedSessionOptions,
  ): Promise<RuntimeSession> => {
    let hydratedSession = options.session

    try {
      await deps.maybeResize?.(options.sessionId)
    } catch (error) {
      deps.logWarning?.("failed to resize attached session", {
        session_id: options.sessionId,
        attachment_id: options.attachmentId,
        error: formatError(error),
      })
    }

    try {
      await deps.catchUpAttachedSession?.(options.sessionId, options.attachmentId, hydratedSession)
    } catch (error) {
      deps.logWarning?.("failed to catch up attached session", {
        session_id: options.sessionId,
        attachment_id: options.attachmentId,
        error: formatError(error),
      })
    }

    try {
      hydratedSession = await deps.getSessionState(options.sessionId)
    } catch (error) {
      deps.logWarning?.("failed to hydrate attached session after attach", {
        session_id: options.sessionId,
        attachment_id: options.attachmentId,
        error: formatError(error),
      })
    }

    try {
      await deps.refreshAgentPanes(hydratedSession)
    } catch (error) {
      deps.logWarning?.("failed to refresh agent panes after attach", {
        session_id: options.sessionId,
        attachment_id: options.attachmentId,
        error: formatError(error),
      })
    }

    return hydratedSession
  }

  return {
    hydrateCurrentAttachedSession,
    finalizeAttachedSessionBinding,
  }
}

type PromptStopAttachment = {
  id: string
}

type PromptStopActivePrompt = {
  target_agent_id?: string | null
}

type PromptStopControllerOptions = {
  getAttachment: () => PromptStopAttachment | null | undefined
  getActivePrompt: () => PromptStopActivePrompt | null | undefined
  getSessionId: () => string
  getFallbackStreamingAgentId: () => string | null
  cancelActivePrompt: (sessionId: string, attachmentId: string) => Promise<void>
  onCancellationRequested: (targetAgentId: string | null) => void
  onCancellationFailed: (error: unknown) => void
}

export type PromptStopController = {
  request(): Promise<boolean>
  reset(): void
  isInFlight(): boolean
}

export function createPromptStopController(
  options: PromptStopControllerOptions,
): PromptStopController {
  let inFlight = false

  return {
    async request() {
      const attachment = options.getAttachment()
      if (inFlight || !options.getActivePrompt() || !attachment) {
        return false
      }

      inFlight = true
      try {
        await options.cancelActivePrompt(options.getSessionId(), attachment.id)
        options.onCancellationRequested(
          options.getActivePrompt()?.target_agent_id ?? options.getFallbackStreamingAgentId(),
        )
        return true
      } catch (error) {
        inFlight = false
        options.onCancellationFailed(error)
        return false
      }
    },
    reset() {
      inFlight = false
    },
    isInFlight() {
      return inFlight
    },
  }
}

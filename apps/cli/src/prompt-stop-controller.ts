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
  cancelActivePrompt: (sessionId: string, attachmentId: string, targetAgentId?: string | null) => Promise<void>
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
      const prompt = options.getActivePrompt()
      if (inFlight || !prompt || !attachment) {
        return false
      }

      // MP-08 / MP-09 / MP-10 / MP-11 A10/G03: preserve this target across I/O.
      const targetAgentId = prompt.target_agent_id ?? options.getFallbackStreamingAgentId()
      inFlight = true
      try {
        await options.cancelActivePrompt(options.getSessionId(), attachment.id, targetAgentId)
        options.onCancellationRequested(targetAgentId)
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

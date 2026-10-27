import type { PendingPromptAttachment } from "./prompt-attachment-state.js"

type PromptAttachmentHighlightInput = {
  plainText: string
  clearAllHighlights: () => void
  addHighlightByCharRange: (range: { start: number; end: number; styleId: number }) => void
}

type PromptAttachmentHighlightControllerOptions = {
  getPromptInput: () => PromptAttachmentHighlightInput | null
  getPendingAttachments: () => readonly PendingPromptAttachment[]
  styleIdForKind: (kind: PendingPromptAttachment["kind"]) => number
}

export type PromptAttachmentHighlightController = {
  refresh(): boolean
}

export function createPromptAttachmentHighlightController(
  options: PromptAttachmentHighlightControllerOptions,
): PromptAttachmentHighlightController {
  return {
    refresh() {
      const promptInput = options.getPromptInput()
      if (!promptInput) {
        return false
      }

      promptInput.clearAllHighlights()
      const value = promptInput.plainText
      for (const file of options.getPendingAttachments()) {
        let start = value.indexOf(file.token)
        while (start !== -1) {
          promptInput.addHighlightByCharRange({
            start,
            end: start + file.token.length,
            styleId: options.styleIdForKind(file.kind),
          })
          start = value.indexOf(file.token, start + file.token.length)
        }
      }
      return true
    },
  }
}

import { clipboardCopyMessage, copyTextToClipboard } from "./clipboard.js"

type ClipboardRenderer = Parameters<typeof copyTextToClipboard>[1]

type SelectionSource = {
  getSelectedText: () => string | null | undefined
  isDragging?: boolean
}

export type ClipboardControllerRenderer = ClipboardRenderer & {
  getSelection: () => SelectionSource | null | undefined
  clearSelection: () => void
}

export type ClipboardPromptInput = {
  plainText: string
  getSelection: () => { start: number; end: number } | null | undefined
}

export type ClipboardControllerDeps = {
  renderer: ClipboardControllerRenderer
  promptInput: () => ClipboardPromptInput | null
  flashFooter: (message: string, tone: "info" | "error") => void
  logWarning?: (message: string, fields?: Record<string, unknown>) => void
  formatError?: (error: unknown) => string
  copyText?: typeof copyTextToClipboard
}

export function createClipboardController(deps: ClipboardControllerDeps) {
  const copyText = deps.copyText ?? copyTextToClipboard
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const copyTextWithFeedback = (text: string | null | undefined) => {
    if (!text) {
      return false
    }
    void copyText(text, deps.renderer)
      .then((result) => {
        deps.flashFooter(clipboardCopyMessage(result), result === "unavailable" ? "error" : "info")
      })
      .catch((error) => {
        deps.logWarning?.("selection copy failed", {
          error: formatError(error),
        })
        deps.flashFooter(clipboardCopyMessage("unavailable"), "error")
      })
    return true
  }

  const copyPromptSelection = () => {
    const input = deps.promptInput()
    const selection = input?.getSelection()
    if (!selection || selection.start === selection.end || !input) {
      return false
    }
    const start = Math.max(0, Math.min(selection.start, selection.end))
    const end = Math.min(input.plainText.length, Math.max(selection.start, selection.end))
    return copyTextWithFeedback(input.plainText.slice(start, end))
  }

  const copySelection = () => {
    const selection = deps.renderer.getSelection()
    if (selection?.isDragging) return false
    // Keep the highlight and native-copy fallback until the next selection/edit.
    return copyTextWithFeedback(selection?.getSelectedText())
  }

  return {
    copyPromptSelection,
    copySelection,
    copyTextWithFeedback,
  }
}

import { isSelectionCopyKey, type SelectionCopyKey } from "./selection-copy-key.js"
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

  const selectedPromptText = () => {
    const input = deps.promptInput()
    const selection = input?.getSelection()
    if (!selection || selection.start === selection.end || !input) {
      return null
    }
    const start = Math.max(0, Math.min(selection.start, selection.end))
    const end = Math.min(input.plainText.length, Math.max(selection.start, selection.end))
    return input.plainText.slice(start, end)
  }

  const selectedTerminalText = () => {
    const selection = deps.renderer.getSelection()
    if (selection?.isDragging) return null
    // Keep the highlight and native-copy fallback until the next selection/edit.
    return selection?.getSelectedText() ?? null
  }

  // MP-08 / MP-10: snapshot at decoded key dispatch, before a later key or
  // paste in the same stdin chunk can replace/extend the selected range.
  const snapshots: Array<{ key: SelectionCopyKey; text: string | null }> = []
  let replayed: { text: string | null } | undefined
  const selectedText = () => selectedPromptText() || selectedTerminalText()
  // MP-08 / MP-10: release transcript selection in decoded event order so
  // a later copy in this chunk cannot snapshot it. Raw routing still owns
  // the deferred rebuild flush; preserve the textarea's own selection.
  const clearRetainedSelection = () => {
    if (!deps.promptInput()?.getSelection()) deps.renderer.clearSelection()
  }

  return {
    captureCopyKey(event: SelectionCopyKey) {
      if (isSelectionCopyKey(event)) snapshots.push({ key: event, text: selectedText() })
      else if (event.name && event.eventType !== "release") clearRetainedSelection()
    },
    capturePaste: clearRetainedSelection,
    replayCopyKey(event: SelectionCopyKey) {
      replayed = undefined
      const next = snapshots[0]
      if (next && next.key.name === event.name && next.key.sequence === event.sequence && isSelectionCopyKey(event)) {
        replayed = snapshots.shift()
      }
    },
    discardCopyInput() { snapshots.length = 0; replayed = undefined },
    copyCapturedSelection: () => copyTextWithFeedback(replayed ? replayed.text : selectedText()),
    copyPromptSelection: () => copyTextWithFeedback(selectedPromptText()),
    copySelection: () => copyTextWithFeedback(selectedTerminalText()),
    copyTextWithFeedback,
  }
}

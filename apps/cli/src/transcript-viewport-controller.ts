export type TranscriptViewportScrollbox = {
  scrollHeight: number
  height: number
  scrollLeft: number
  scrollTop: number
  scrollTo(position: { x: number; y: number }): void
  requestRender(): void
}

export type TranscriptViewportControllerDeps = {
  getScrollbox: () => TranscriptViewportScrollbox | null | undefined
  cancelHistoryScrollRestore: () => void
  setLastTranscriptScrollTop: (scrollTop: number) => void
}

export function createTranscriptViewportController(
  deps: TranscriptViewportControllerDeps,
) {
  const scrollToBottom = () => {
    const scrollbox = deps.getScrollbox()
    if (!scrollbox) {
      return false
    }
    deps.cancelHistoryScrollRestore()
    const maxScrollTop = Math.max(0, scrollbox.scrollHeight - scrollbox.height)
    scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: maxScrollTop })
    scrollbox.requestRender()
    deps.setLastTranscriptScrollTop(scrollbox.scrollTop)
    return true
  }

  return {
    scrollToBottom,
  }
}

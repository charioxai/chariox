export type TranscriptScrollboxRefRenderable = {
  scrollLeft: number
  scrollTop: number
  scrollTo(position: { x: number; y: number }): void
  requestRender(): void
  remove(renderableId: string): void
}

export type TranscriptScrollState = {
  left: number
  top: number
}

export function createTranscriptScrollboxRefController<TScrollbox extends TranscriptScrollboxRefRenderable>() {
  let scrollbox: TScrollbox | undefined

  return {
    assignScrollbox(value: TScrollbox | undefined) {
      scrollbox = value
    },
    current() {
      return scrollbox
    },
    hasScrollbox() {
      return Boolean(scrollbox)
    },
    scrollTop(fallback: number) {
      return scrollbox?.scrollTop ?? fallback
    },
    scrollState(): TranscriptScrollState | null {
      return scrollbox ? { left: scrollbox.scrollLeft, top: scrollbox.scrollTop } : null
    },
    scrollTo(position: { x: number; y: number }) {
      scrollbox?.scrollTo(position)
    },
    requestRender() {
      scrollbox?.requestRender()
    },
    remove(renderableId: string) {
      if (!scrollbox) {
        return false
      }
      scrollbox.remove(renderableId)
      return true
    },
  }
}

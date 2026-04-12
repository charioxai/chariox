import type { TranscriptEntry } from "./cli-types.js"

export type TranscriptPaneRenderable = {
  entry: TranscriptEntry
  wrapper: {
    id: string
    destroyRecursively: () => void
  }
}

export type TranscriptPaneScrollbox = {
  scrollTop: number
  scrollLeft: number
  scrollHeight: number
  height: number
  remove: (id: string) => unknown
  scrollTo: (position: { x: number; y: number }) => unknown
  requestRender: () => unknown
}

export function transcriptEntriesEqual(left: TranscriptEntry, right: TranscriptEntry) {
  return left.id === right.id
    && left.role === right.role
    && left.text === right.text
    && left.sourceText === right.sourceText
    && left.emphasis === right.emphasis
    && left.hidden === right.hidden
    && left.toggleMode === right.toggleMode
    && left.blobCollapsible === right.blobCollapsible
    && left.blobCollapsed === right.blobCollapsed
    && left.blobTitle === right.blobTitle
    && left.blobSummary === right.blobSummary
}

export function transcriptEntriesShareMountedPrefix(left: TranscriptEntry, right: TranscriptEntry) {
  if (left.role !== right.role) {
    return false
  }
  if (left.role === "turn_toggle") {
    return left.turnId === right.turnId
      && left.toggleMode === right.toggleMode
      && left.text === right.text
  }
  return transcriptEntriesEqual(left, right)
}

export function reconcileMountedTranscriptPane(options: {
  scrollbox: TranscriptPaneScrollbox | undefined
  currentEntries: TranscriptEntry[]
  nextEntries: TranscriptEntry[]
  renderables: Map<number, TranscriptPaneRenderable>
  clampScrollTop: (scrollTop: number, scrollHeight: number, viewportHeight: number) => number
  rebuild: () => void
  removeEmptyRenderable?: () => void
  mountEntry: (entry: TranscriptEntry, requestRender: boolean) => void
  onScrollTop?: (scrollTop: number) => void
}) {
  const {
    scrollbox,
    currentEntries,
    nextEntries,
    renderables,
    clampScrollTop,
    rebuild,
    removeEmptyRenderable,
    mountEntry,
    onScrollTop,
  } = options

  if (!scrollbox || nextEntries.length === 0) {
    rebuild()
    return
  }

  removeEmptyRenderable?.()

  const previousScrollTop = scrollbox.scrollTop
  const previousVisibleEntries = currentEntries.filter((entry) => !entry.hidden && !entry.historyDeferred)
  const nextVisibleEntries = nextEntries.filter((entry) => !entry.hidden && !entry.historyDeferred)

  let preservedPrefixLength = 0
  while (
    preservedPrefixLength < previousVisibleEntries.length
    && preservedPrefixLength < nextVisibleEntries.length
    && transcriptEntriesShareMountedPrefix(
      previousVisibleEntries[preservedPrefixLength]!,
      nextVisibleEntries[preservedPrefixLength]!,
    )
  ) {
    const previousEntry = previousVisibleEntries[preservedPrefixLength]!
    const nextEntry = nextVisibleEntries[preservedPrefixLength]!
    const renderable = renderables.get(previousEntry.id)
    if (renderable) {
      if (previousEntry.id !== nextEntry.id) {
        renderables.delete(previousEntry.id)
        renderables.set(nextEntry.id, renderable)
      }
      renderable.entry = nextEntry
    }
    preservedPrefixLength += 1
  }

  for (const entry of previousVisibleEntries.slice(preservedPrefixLength)) {
    const renderable = renderables.get(entry.id)
    if (!renderable) {
      continue
    }
    scrollbox.remove(renderable.wrapper.id)
    renderable.wrapper.destroyRecursively()
    renderables.delete(entry.id)
  }

  for (const entry of nextVisibleEntries.slice(preservedPrefixLength)) {
    mountEntry(entry, false)
  }

  scrollbox.scrollTo({
    x: scrollbox.scrollLeft,
    y: clampScrollTop(previousScrollTop, scrollbox.scrollHeight, scrollbox.height),
  })
  onScrollTop?.(scrollbox.scrollTop)
  scrollbox.requestRender()
}

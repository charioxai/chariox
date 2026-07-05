export type TranscriptPaneEntry = {
  readonly id: number
  readonly role: string
  readonly text: string
  readonly sourceText?: string
  readonly emphasis?: string
  readonly hidden?: boolean
  readonly historyDeferred?: boolean
  readonly turnId?: number | null
  readonly toggleMode?: "expand" | "collapse"
  readonly blobCollapsible?: boolean
  readonly blobCollapsed?: boolean
  readonly blobTitle?: string
  readonly blobSummary?: string
}

export type TranscriptPaneRenderable<TEntry extends TranscriptPaneEntry> = {
  entry: TEntry
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

export function transcriptEntriesEqual<TEntry extends TranscriptPaneEntry>(
  left: TEntry,
  right: TEntry,
): boolean {
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

export function transcriptEntriesShareMountedPrefix<TEntry extends TranscriptPaneEntry>(
  left: TEntry,
  right: TEntry,
): boolean {
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

export function reconcileMountedTranscriptPane<TEntry extends TranscriptPaneEntry>(options: {
  readonly scrollbox: TranscriptPaneScrollbox | undefined
  readonly currentEntries: readonly TEntry[]
  readonly nextEntries: readonly TEntry[]
  readonly renderables: Map<number, TranscriptPaneRenderable<TEntry>>
  readonly clampScrollTop: (scrollTop: number, scrollHeight: number, viewportHeight: number) => number
  readonly rebuild: () => void
  readonly removeEmptyRenderable?: () => void
  readonly mountEntry: (entry: TEntry, requestRender: boolean) => void
  readonly onScrollTop?: (scrollTop: number) => void
}): void {
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
  const previousVisibleEntries = currentEntries.filter(transcriptPaneEntryIsMounted)
  const nextVisibleEntries = nextEntries.filter(transcriptPaneEntryIsMounted)

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

function transcriptPaneEntryIsMounted<TEntry extends TranscriptPaneEntry>(
  entry: TEntry,
): boolean {
  return !entry.hidden && !entry.historyDeferred
}

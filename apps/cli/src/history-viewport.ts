export type HistoryFragmentEntry = {
  id: number
  historyEntryIndex?: number
  historyFragmentStart?: number
  historyFragmentEnd?: number
}

export function findPrependedHistoryMergedHeadId(
  olderEntries: HistoryFragmentEntry[],
  currentEntries: HistoryFragmentEntry[],
) {
  if (olderEntries.length === 0 || currentEntries.length === 0) {
    return null
  }

  const tail = olderEntries.at(-1)
  const head = currentEntries[0]
  if (
    tail?.historyEntryIndex === undefined
    || head?.historyEntryIndex === undefined
    || tail.historyEntryIndex !== head.historyEntryIndex
    || tail.historyFragmentEnd !== head.historyFragmentStart
  ) {
    return null
  }

  return head.id
}

export function computePrependedHistoryScrollTop(
  previousScrollTop: number,
  previousScrollHeight: number,
  nextScrollHeight: number,
  viewportHeight: number,
) {
  const heightDelta = Math.max(0, nextScrollHeight - previousScrollHeight)
  return clampScrollTop(previousScrollTop + heightDelta, nextScrollHeight, viewportHeight)
}

export function computeAnchoredScrollTop(
  anchorViewportOffset: number | null,
  nextAnchorY: number | null,
  nextScrollHeight: number,
  viewportHeight: number,
) {
  if (anchorViewportOffset === null || nextAnchorY === null) {
    return null
  }
  return clampScrollTop(nextAnchorY - anchorViewportOffset, nextScrollHeight, viewportHeight)
}

export function computeCollapsedHistoryScrollTop(
  previousScrollTop: number,
  previousScrollHeight: number,
  nextScrollHeight: number,
  viewportHeight: number,
) {
  const heightDelta = Math.max(0, previousScrollHeight - nextScrollHeight)
  return clampScrollTop(previousScrollTop - heightDelta, nextScrollHeight, viewportHeight)
}

export function clampScrollTop(
  scrollTop: number,
  scrollHeight: number,
  viewportHeight: number,
) {
  const maxScrollTop = Math.max(0, scrollHeight - viewportHeight)
  return Math.max(0, Math.min(scrollTop, maxScrollTop))
}

export function findTurnPromptScrollTarget(
  promptOffsets: number[],
  scrollTop: number,
  direction: "previous" | "next",
) {
  if (promptOffsets.length === 0) {
    return null
  }

  if (promptOffsets.length === 1) {
    return promptOffsets[0]
  }

  // Find which prompt is currently in view (at or just above the scroll position)
  let currentIndex = 0
  for (let i = 0; i < promptOffsets.length; i++) {
    const offset = promptOffsets[i]
    if (offset !== undefined && offset <= scrollTop + 5) {
      currentIndex = i
    } else {
      break
    }
  }

  if (direction === "previous") {
    // Already at the first prompt, stay there
    const prevIndex = Math.max(0, currentIndex - 1)
    return promptOffsets[prevIndex] ?? null
  }

  // direction === "next"
  // Already at the last prompt, stay there
  const nextIndex = Math.min(promptOffsets.length - 1, currentIndex + 1)
  return promptOffsets[nextIndex] ?? null
}

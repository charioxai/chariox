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
  const maxScrollTop = Math.max(0, nextScrollHeight - viewportHeight)
  return Math.max(0, Math.min(previousScrollTop + heightDelta, maxScrollTop))
}

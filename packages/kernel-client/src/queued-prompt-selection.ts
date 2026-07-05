export type QueuedPromptSelectionItem = {
  readonly promptId: string
}

export function selectedQueuedPromptIndex(
  items: readonly QueuedPromptSelectionItem[],
  selectedPromptId: string | null | undefined,
): number {
  if (items.length === 0) {
    return -1
  }
  const selectedIndex = selectedPromptId
    ? items.findIndex((item) => item.promptId === selectedPromptId)
    : -1
  return selectedIndex >= 0 ? selectedIndex : 0
}

export function selectedQueuedPromptId(
  items: readonly QueuedPromptSelectionItem[],
  selectedPromptId: string | null | undefined,
): string | null {
  const index = selectedQueuedPromptIndex(items, selectedPromptId)
  return index >= 0 ? items[index]?.promptId ?? null : null
}

export function nextQueuedPromptSelectionId(
  items: readonly QueuedPromptSelectionItem[],
  selectedPromptId: string | null | undefined,
  delta: number,
): string | null {
  if (items.length === 0) {
    return null
  }
  const currentIndex = selectedQueuedPromptIndex(items, selectedPromptId)
  const nextIndex = (currentIndex + delta + items.length) % items.length
  return items[nextIndex]?.promptId ?? null
}

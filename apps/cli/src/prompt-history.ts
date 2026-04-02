export type PromptHistoryDirection = "previous" | "next"

export type PromptHistoryNavigation = {
  entries: readonly string[]
  currentText: string
  navigationIndex: number | null
  navigationDraft: string | null
  direction: PromptHistoryDirection
}

export type PromptHistoryNavigationResult = {
  text: string
  navigationIndex: number | null
  navigationDraft: string | null
}

const PROMPT_HISTORY_LIMIT = 100

export function pushPromptHistoryEntry(
  entries: readonly string[],
  prompt: string,
): string[] {
  const normalized = prompt.trimEnd()
  if (!normalized) {
    return [...entries]
  }
  const nextEntries = entries.at(-1) === normalized
    ? [...entries]
    : [...entries, normalized]
  return nextEntries.slice(-PROMPT_HISTORY_LIMIT)
}

export function navigatePromptHistory(
  options: PromptHistoryNavigation,
): PromptHistoryNavigationResult {
  const { entries, currentText, navigationIndex, navigationDraft, direction } = options
  if (entries.length === 0) {
    return {
      text: currentText,
      navigationIndex,
      navigationDraft,
    }
  }

  if (direction === "previous") {
    if (navigationIndex === null) {
      return {
        text: entries[entries.length - 1] ?? "",
        navigationIndex: entries.length - 1,
        navigationDraft: currentText,
      }
    }
    return {
      text: entries[Math.max(0, navigationIndex - 1)] ?? "",
      navigationIndex: Math.max(0, navigationIndex - 1),
      navigationDraft,
    }
  }

  if (navigationIndex === null) {
    return {
      text: currentText,
      navigationIndex,
      navigationDraft,
    }
  }

  if (navigationIndex >= entries.length - 1) {
    return {
      text: navigationDraft ?? "",
      navigationIndex: null,
      navigationDraft: null,
    }
  }

  return {
    text: entries[navigationIndex + 1] ?? "",
    navigationIndex: navigationIndex + 1,
    navigationDraft,
  }
}

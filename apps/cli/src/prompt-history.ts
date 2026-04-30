import type { PromptInputHistoryEntry, SessionHistoryPageEntry } from "./cli-types.js"

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

export type PromptHistoryKeyPolicy = {
  attached: boolean
  promptFocused: boolean
  commandCenterOpen: boolean
  keyName: string
  currentText?: string | undefined
  cursorOffset?: number | undefined
  eventType?: string | undefined
  ctrl?: boolean | undefined
  meta?: boolean | undefined
  alt?: boolean | undefined
  shift?: boolean | undefined
}

export type PromptContentChangePolicy = {
  currentText: string
  previousSnapshot: string
  programmaticMutation: boolean
  dropPending: boolean
}

export function isProgrammaticPromptContentEcho(
  options: PromptContentChangePolicy,
) {
  return options.programmaticMutation
    || options.dropPending
    || options.currentText === options.previousSnapshot
}

export function promptHistoryDirectionForKey(
  options: PromptHistoryKeyPolicy,
): PromptHistoryDirection | null {
  if (
    !options.attached
    || !options.promptFocused
    || options.commandCenterOpen
    || options.eventType === "release"
    || options.ctrl
    || options.meta
    || options.alt
    || options.shift
  ) {
    return null
  }
  if (options.keyName === "up") {
    if (!cursorIsOnFirstPromptLine(options.currentText ?? "", options.cursorOffset ?? 0)) {
      return null
    }
    return "previous"
  }
  if (options.keyName === "down") {
    if (!cursorIsOnLastPromptLine(options.currentText ?? "", options.cursorOffset ?? 0)) {
      return null
    }
    return "next"
  }
  return null
}

export function cursorIsOnFirstPromptLine(text: string, cursorOffset: number): boolean {
  const offset = boundedCursorOffset(text, cursorOffset)
  return !text.slice(0, offset).includes("\n")
}

export function cursorIsOnLastPromptLine(text: string, cursorOffset: number): boolean {
  const offset = boundedCursorOffset(text, cursorOffset)
  return !text.slice(offset).includes("\n")
}

function boundedCursorOffset(text: string, cursorOffset: number): number {
  if (!Number.isFinite(cursorOffset)) {
    return text.length
  }
  return Math.max(0, Math.min(text.length, Math.floor(cursorOffset)))
}

export function pushPromptHistoryEntry(
  entries: readonly string[],
  prompt: string,
): string[] {
  const normalized = prompt.trimEnd()
  if (!normalized) {
    return [...entries]
  }
  return entries.at(-1) === normalized
    ? [...entries]
    : [...entries, normalized]
}

export function extractPromptHistoryEntries(
  historyEntries: SessionHistoryPageEntry[],
): string[] {
  const mergedEntries = mergeAdjacentPromptHistoryEntries(historyEntries)
  let prompts: string[] = []
  for (const entry of mergedEntries) {
    if (entry.entry.kind !== "user_prompt") {
      continue
    }
    prompts = pushPromptHistoryEntry(prompts, entry.entry.text)
  }
  return prompts
}

export function extractPromptInputHistoryEntries(
  historyEntries: readonly PromptInputHistoryEntry[],
): string[] {
  let prompts: string[] = []
  for (const entry of [...historyEntries].sort((left, right) => left.sequence - right.sequence)) {
    prompts = pushPromptHistoryEntry(prompts, entry.text)
  }
  return prompts
}

function mergeAdjacentPromptHistoryEntries(historyEntries: SessionHistoryPageEntry[]) {
  const merged: SessionHistoryPageEntry[] = []

  for (const entry of historyEntries) {
    const previous = merged.at(-1)
    if (
      previous
      && previous.entry_index === entry.entry_index
      && previous.entry.kind === entry.entry.kind
      && previous.fragment_end === entry.fragment_start
    ) {
      previous.fragment_end = entry.fragment_end
      previous.entry.text += entry.entry.text
      previous.total_chars = Math.max(previous.total_chars, entry.total_chars)
      continue
    }

    merged.push({
      entry_index: entry.entry_index,
      fragment_start: entry.fragment_start,
      fragment_end: entry.fragment_end,
      total_chars: entry.total_chars,
      entry: {
        kind: entry.entry.kind,
        text: entry.entry.text,
        ...(entry.entry.agent_id !== undefined ? { agent_id: entry.entry.agent_id } : {}),
        ...(entry.entry.provider_run_id !== undefined ? { provider_run_id: entry.entry.provider_run_id } : {}),
        ...(entry.entry.merge_key !== undefined ? { merge_key: entry.entry.merge_key } : {}),
      },
    })
  }

  return merged
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
    if (navigationDraft !== null) {
      return {
        text: navigationDraft,
        navigationIndex: null,
        navigationDraft: null,
      }
    }
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

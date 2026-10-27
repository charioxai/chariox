import type { PromptInputHistoryEntry, SessionHistoryPageEntry } from "./kernel-types.js"
import { mergeAdjacentSessionHistoryPageEntries } from "./session-history-page-entries.js"

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

export type PromptHistoryNavigationKeyPolicy = PromptHistoryKeyPolicy & {
  navigationIndex: number | null
  navigationDraft: string | null
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

export function resolvePromptHistoryKeyNavigation(
  options: PromptHistoryNavigationKeyPolicy,
): PromptHistoryDirection | null {
  const direction = promptHistoryDirectionForKey(options)
  if (!direction) {
    return null
  }
  if (direction === "next" && options.navigationIndex === null && options.navigationDraft === null) {
    return null
  }
  return direction
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
  historyEntries: readonly SessionHistoryPageEntry[],
): string[] {
  const mergedEntries = mergeAdjacentSessionHistoryPageEntries(historyEntries)
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

export function maxPromptInputHistorySequence(entries: readonly PromptInputHistoryEntry[]) {
  return entries.reduce((max, entry) => Math.max(max, entry.sequence), 0)
}

export function promptHistoryEntryListsEqual(left: readonly string[], right: readonly string[]) {
  return left.length === right.length && left.every((entry, index) => entry === right[index])
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

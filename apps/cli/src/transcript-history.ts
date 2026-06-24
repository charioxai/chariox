import type { SessionHistoryEntry, SessionHistoryPageEntry, TranscriptEntry } from "./cli-types.js"
import {
  formatToolTranscriptUpdate,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  shouldRenderProviderStatus,
  type ToolTranscriptUpdate,
} from "./transcript.js"
import { trimSingleTrailingNewline } from "./transcript-text.js"

function shouldDeferHistoryEntry(entry: TranscriptEntry) {
  return entry.historyFragmentStart !== undefined && entry.historyFragmentStart > 0
}

export function applyHistoryDeferral(entry: TranscriptEntry) {
  const deferred = shouldDeferHistoryEntry(entry)
  if (deferred) {
    entry.historyDeferred = true
  } else {
    delete entry.historyDeferred
  }
  return entry
}

export function markDeferredHistoryEntries(items: TranscriptEntry[]) {
  if (items.length === 0) {
    return items
  }
  return items.map((entry, index) => {
    if (index === 0) {
      return applyHistoryDeferral({ ...entry })
    }
    if (!entry.historyDeferred) {
      return entry
    }
    const next = { ...entry }
    delete next.historyDeferred
    return next
  })
}

export function mergePrependedHistoryFragments(older: TranscriptEntry, newer: TranscriptEntry): TranscriptEntry {
  const sourceText = (older.sourceText ?? older.text) + (newer.sourceText ?? newer.text)
  const mergedBase: TranscriptEntry = {
    ...newer,
    text: newer.text,
    sourceText,
  }
  if (older.historyFragmentStart !== undefined) mergedBase.historyFragmentStart = older.historyFragmentStart
  if (newer.historyFragmentEnd !== undefined) mergedBase.historyFragmentEnd = newer.historyFragmentEnd
  const totalChars = newer.historyTotalChars ?? older.historyTotalChars
  if (totalChars !== undefined) mergedBase.historyTotalChars = totalChars
  if (older.role !== "tool") {
    return applyHistoryDeferral({
      ...mergedBase,
      text: older.text + newer.text,
    })
  }

  const parsed = parseToolTranscriptUpdate(sourceText)
  if (!parsed) {
    const pending: TranscriptEntry = {
      ...mergedBase,
      text: sourceText,
    }
    delete pending.mergeKey
    return {
      ...applyHistoryDeferral(pending),
    }
  }

  const merged = mergeToolTranscriptUpdate(null, parsed)
  return applyHistoryDeferral({
    ...mergedBase,
    text: formatToolTranscriptUpdate(merged),
    mergeKey: parsed.id,
  })
}

export function stitchPrependedHistory(olderEntries: TranscriptEntry[], currentEntries: TranscriptEntry[]) {
  if (olderEntries.length === 0 || currentEntries.length === 0) {
    return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
  }

  const tail = olderEntries.at(-1)
  const head = currentEntries[0]
  if (
    tail?.historyEntryIndex === undefined
    || head?.historyEntryIndex === undefined
    || tail.historyEntryIndex !== head.historyEntryIndex
    || tail.historyFragmentEnd !== head.historyFragmentStart
  ) {
    return markDeferredHistoryEntries([...olderEntries, ...currentEntries])
  }

  return markDeferredHistoryEntries([
    ...olderEntries.slice(0, -1),
    mergePrependedHistoryFragments(tail, head),
    ...currentEntries.slice(1),
  ])
}

export function mergeAdjacentHistoryPageEntries(historyEntries: SessionHistoryPageEntry[]) {
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
        ...(entry.entry.source !== undefined ? { source: entry.entry.source } : {}),
        ...(entry.entry.external_provider !== undefined ? { external_provider: entry.entry.external_provider } : {}),
        ...(entry.entry.external_provider_session_id !== undefined ? { external_provider_session_id: entry.entry.external_provider_session_id } : {}),
        ...(entry.entry.external_provider_turn_id !== undefined ? { external_provider_turn_id: entry.entry.external_provider_turn_id } : {}),
        ...(entry.entry.observed_at_ms !== undefined ? { observed_at_ms: entry.entry.observed_at_ms } : {}),
        ...(entry.entry.external_observation !== undefined ? { external_observation: entry.entry.external_observation } : {}),
      },
    })
  }

  return merged
}

export function hydrateTranscriptEntries(historyEntries: SessionHistoryPageEntry[]): TranscriptEntry[] {
  const mergedHistoryEntries = mergeAdjacentHistoryPageEntries(historyEntries)
  const entries: TranscriptEntry[] = []
  const tools = new Map<string, ToolTranscriptUpdate>()
  let nextId = 0
  let currentTurnId = 0

  const appendTranscriptEntry = (
    role: TranscriptEntry["role"],
    chunk: string,
    options: {
      mergeKey?: string
      sourceText?: string
      source?: TranscriptEntry["source"]
      externalProvider?: string | null
      externalProviderSessionId?: string | null
      externalProviderTurnId?: string | null
      observedAtMs?: number | null
      emphasis?: TranscriptEntry["emphasis"]
      historyEntryIndex?: number
      historyFragmentStart?: number
      historyFragmentEnd?: number
      historyTotalChars?: number
      turnId?: number
    } = {},
  ) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n")
    if (!normalized) {
      return
    }

    if (options.mergeKey) {
      for (let index = entries.length - 1; index >= 0; index -= 1) {
        const candidate = entries[index]
        if (candidate?.role === role && candidate.mergeKey === options.mergeKey) {
          if (role === "assistant" || role === "reasoning") {
            candidate.text += normalized
            if (options.sourceText !== undefined) {
              candidate.sourceText = `${candidate.sourceText ?? ""}${options.sourceText}`
            }
          } else {
            candidate.text = normalized
            if (options.sourceText !== undefined) candidate.sourceText = options.sourceText
          }
          if (options.emphasis !== undefined) candidate.emphasis = options.emphasis
          if (options.source !== undefined) candidate.source = options.source
          if (options.externalProvider !== undefined) candidate.externalProvider = options.externalProvider
          if (options.externalProviderSessionId !== undefined) candidate.externalProviderSessionId = options.externalProviderSessionId
          if (options.externalProviderTurnId !== undefined) candidate.externalProviderTurnId = options.externalProviderTurnId
          if (options.observedAtMs !== undefined) candidate.observedAtMs = options.observedAtMs
          if (options.historyEntryIndex !== undefined) candidate.historyEntryIndex = options.historyEntryIndex
          if (options.historyFragmentStart !== undefined) candidate.historyFragmentStart = options.historyFragmentStart
          if (options.historyFragmentEnd !== undefined) candidate.historyFragmentEnd = options.historyFragmentEnd
          if (options.historyTotalChars !== undefined) candidate.historyTotalChars = options.historyTotalChars
          return
        }
      }
    }

    const last = entries.at(-1)
    if (!options.mergeKey && last?.role === role && (role === "assistant" || role === "reasoning")) {
      last.text += normalized
      return
    }

    nextId += 1
    const nextEntry: TranscriptEntry = { id: nextId, role, text: normalized }
    if (options.mergeKey) {
      nextEntry.mergeKey = options.mergeKey
    }
    if (options.sourceText !== undefined) nextEntry.sourceText = options.sourceText
    if (options.source !== undefined) nextEntry.source = options.source
    if (options.externalProvider !== undefined) nextEntry.externalProvider = options.externalProvider
    if (options.externalProviderSessionId !== undefined) nextEntry.externalProviderSessionId = options.externalProviderSessionId
    if (options.externalProviderTurnId !== undefined) nextEntry.externalProviderTurnId = options.externalProviderTurnId
    if (options.observedAtMs !== undefined) nextEntry.observedAtMs = options.observedAtMs
    if (options.emphasis !== undefined) nextEntry.emphasis = options.emphasis
    if (options.historyEntryIndex !== undefined) nextEntry.historyEntryIndex = options.historyEntryIndex
    if (options.historyFragmentStart !== undefined) nextEntry.historyFragmentStart = options.historyFragmentStart
    if (options.historyFragmentEnd !== undefined) nextEntry.historyFragmentEnd = options.historyFragmentEnd
    if (options.historyTotalChars !== undefined) nextEntry.historyTotalChars = options.historyTotalChars
    if (options.turnId !== undefined) nextEntry.turnId = options.turnId
    entries.push(nextEntry)
  }

  for (const pageEntry of mergedHistoryEntries) {
    const options: {
      historyEntryIndex: number
      historyFragmentStart: number
      historyFragmentEnd: number
      historyTotalChars: number
      turnId?: number
    } = {
      historyEntryIndex: pageEntry.entry_index,
      historyFragmentStart: pageEntry.fragment_start,
      historyFragmentEnd: pageEntry.fragment_end,
      historyTotalChars: pageEntry.total_chars,
    }
    if (currentTurnId > 0) options.turnId = currentTurnId
    const observedOptions = externalProviderObservedOptions(pageEntry.entry)
    switch (pageEntry.entry.kind) {
      case "user_prompt":
        currentTurnId = Math.max(currentTurnId + 1, (pageEntry.entry_index ?? 0) + 1)
        appendTranscriptEntry("user", trimSingleTrailingNewline(pageEntry.entry.text), {
          ...options,
          ...observedOptions,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
          turnId: currentTurnId,
        })
        break
      case "provider_reasoning":
        appendTranscriptEntry("reasoning", pageEntry.entry.text, {
          ...options,
          ...observedOptions,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
        })
        break
      case "provider_tool": {
        const parsed = parseToolTranscriptUpdate(pageEntry.entry.text)
        if (!parsed) {
          appendTranscriptEntry("tool", pageEntry.entry.text, {
            ...options,
            sourceText: pageEntry.entry.text,
          })
          break
        }
        const merged = mergeToolTranscriptUpdate(tools.get(parsed.id) ?? null, parsed)
        tools.set(parsed.id, merged)
        appendTranscriptEntry("tool", formatToolTranscriptUpdate(merged), {
          ...options,
          mergeKey: parsed.id,
          sourceText: pageEntry.entry.text,
        })
        break
      }
      case "provider_error":
        appendTranscriptEntry("error", pageEntry.entry.text, {
          ...options,
          ...observedOptions,
          emphasis: "error",
        })
        break
      case "provider_status":
        if (shouldRenderProviderStatus(pageEntry.entry.text)) {
          appendTranscriptEntry("status", pageEntry.entry.text, {
            ...options,
            mergeKey: "__provider_status__",
          })
        }
        break
      case "notice":
        appendTranscriptEntry("notice", pageEntry.entry.text, {
          ...options,
          ...observedOptions,
        })
        break
      default:
        appendTranscriptEntry("assistant", pageEntry.entry.text, {
          ...options,
          ...observedOptions,
          ...(pageEntry.entry.merge_key ? { mergeKey: pageEntry.entry.merge_key } : {}),
        })
        break
    }
  }

  return markDeferredHistoryEntries(entries)
}

function externalProviderObservedOptions(entry: SessionHistoryEntry): Partial<TranscriptEntry> {
  if (entry.source !== "external_provider_observed") {
    return {}
  }
  return {
    source: entry.source,
    externalProvider: entry.external_provider ?? null,
    externalProviderSessionId: entry.external_provider_session_id ?? null,
    externalProviderTurnId: entry.external_provider_turn_id ?? null,
    observedAtMs: entry.observed_at_ms ?? null,
  }
}

export function previewLineForHistoryEntry(entry: SessionHistoryEntry) {
  const text = entry.text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  if (!text) {
    return null
  }
  const label = entry.kind === "user_prompt"
    ? "You"
    : entry.kind === "provider_reasoning"
      ? "Think"
      : entry.kind === "provider_tool"
        ? "Tool"
        : entry.kind === "provider_error"
          ? "Err"
          : entry.kind === "provider_status"
            ? "Stat"
            : entry.kind === "notice"
              ? "Note"
              : "Asst"
  return `${label}: ${text.split("\n")[0]}`
}

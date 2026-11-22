import type {
  SessionHistoryEntry,
  SessionHistoryPageEntry,
  TerminalOutputRecord,
  TranscriptEntry,
} from "./kernel-types.js"
import { externalProviderObservedProviderStatusShouldRender } from "./external-provider-observation.js"
import { sessionHistoryEntryKindTranscriptRole } from "./session-history-outline.js"
import { mergeAdjacentSessionHistoryPageEntries } from "./session-history-page-entries.js"
import { providerTranscriptRoleForKind } from "./transcript-kind-role.js"
import { PROVIDER_TERMINAL_OUTPUT_KIND } from "./terminal-record-transcript.js"

export const DEFAULT_TRANSCRIPT_PREVIEW_LINE_LIMIT = 14

export function appendTranscriptPreviewLine(
  current: string,
  line: string,
  limit = DEFAULT_TRANSCRIPT_PREVIEW_LINE_LIMIT,
): string {
  const combined = current ? `${current}\n${line}` : line
  return trimPreviewLines(combined.split("\n"), limit).join("\n")
}

export function formatSessionHistoryPreview(
  historyEntries: readonly SessionHistoryPageEntry[],
  limit = DEFAULT_TRANSCRIPT_PREVIEW_LINE_LIMIT,
): string {
  const lines = mergeAdjacentSessionHistoryPageEntries(historyEntries)
    .map((item) => previewLineForSessionHistoryEntry(item.entry))
    .filter((line): line is string => Boolean(line))
  return trimPreviewLines(lines, limit).join("\n")
}

export function formatTranscriptPreview(
  transcriptEntries: readonly Pick<TranscriptEntry, "role" | "text" | "hidden">[],
  limit = DEFAULT_TRANSCRIPT_PREVIEW_LINE_LIMIT,
): string {
  const lines = transcriptEntries
    .filter((entry) => entry && !entry.hidden)
    .map(previewLineForTranscriptEntry)
    .filter((line): line is string => Boolean(line))
  return trimPreviewLines(lines, limit).join("\n")
}

export function previewLineForSessionHistoryEntry(entry: SessionHistoryEntry): string | null {
  const text = firstNormalizedLine(entry.text)
  if (!text) {
    return null
  }
  if (entry.kind === "provider_status" && !externalProviderObservedProviderStatusShouldRender(entry)) {
    return null
  }
  return `${sessionHistoryEntryPreviewLabel(entry.kind)}: ${text}`
}

export function previewLineForTranscriptEntry(
  entry: Pick<TranscriptEntry, "role" | "text" | "hidden">,
): string | null {
  if (entry.hidden || entry.role === "turn_toggle") {
    return null
  }
  const text = firstNormalizedLine(entry.text)
  if (!text) {
    return null
  }
  return `${transcriptEntryPreviewLabel(entry.role)}: ${text}`
}

export function previewLineForTerminalRecord(
  kind: TerminalOutputRecord["kind"],
  text: string,
): string {
  if (kind === PROVIDER_TERMINAL_OUTPUT_KIND) {
    return ""
  }
  const normalized = firstNormalizedLine(text)
  if (!normalized) {
    return ""
  }
  const label = kind === "prompt_echo"
    ? "You"
    : transcriptEntryPreviewLabel(terminalRecordPreviewRole(kind))
  return `${label}: ${normalized}`
}

export function sessionHistoryEntryPreviewLabel(kind: SessionHistoryEntry["kind"]): string {
  return transcriptEntryPreviewLabel(sessionHistoryEntryKindTranscriptRole(kind))
}

export function transcriptEntryPreviewLabel(role: TranscriptEntry["role"]): string {
  switch (role) {
    case "user":
      return "You"
    case "reasoning":
      return "Think"
    case "tool":
      return "Tool"
    case "error":
      return "Err"
    case "status":
      return "Stat"
    case "notice":
      return "Note"
    case "assistant":
      return "Asst"
    case "turn_toggle":
      return "Turn"
  }
}

function firstNormalizedLine(text: string): string | null {
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  return normalized ? normalized.split("\n")[0] ?? null : null
}

function terminalRecordPreviewRole(kind: TerminalOutputRecord["kind"]): TranscriptEntry["role"] {
  if (kind === "prompt_echo") {
    return "user"
  }
  return providerTranscriptRoleForKind(kind) ?? "assistant"
}

function trimPreviewLines(lines: readonly string[], limit: number): string[] {
  const normalizedLimit = Number.isFinite(limit) ? Math.max(0, Math.trunc(limit)) : DEFAULT_TRANSCRIPT_PREVIEW_LINE_LIMIT
  return normalizedLimit === 0 ? [] : lines.slice(-normalizedLimit)
}

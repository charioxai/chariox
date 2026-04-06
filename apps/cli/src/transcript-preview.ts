import type {
  SessionHistoryPageEntry,
  TerminalOutputRecord,
  TranscriptEntry,
} from "./cli-types.js"
import {
  mergeAdjacentHistoryPageEntries,
  previewLineForHistoryEntry,
} from "./transcript-history.js"

export function appendPreviewLine(current: string, line: string) {
  const combined = current ? `${current}\n${line}` : line
  const lines = combined.split("\n")
  return lines.slice(-14).join("\n")
}

export function formatHistoryPreview(historyEntries: SessionHistoryPageEntry[]) {
  const lines = mergeAdjacentHistoryPageEntries(historyEntries)
    .map((item) => previewLineForHistoryEntry(item.entry))
    .filter(Boolean) as string[]
  return lines.slice(-14).join("\n")
}

export function formatTranscriptPreview(transcriptEntries: TranscriptEntry[]) {
  const lines = transcriptEntries
    .filter((entry) => entry && !entry.hidden)
    .map(previewLineForTranscriptEntry)
    .filter(Boolean) as string[]
  return lines.slice(-14).join("\n")
}

function previewLineForTranscriptEntry(entry: TranscriptEntry) {
  const text = entry.text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  if (!text || entry.role === "turn_toggle") {
    return null
  }
  const label = entry.role === "user"
    ? "You"
    : entry.role === "reasoning"
      ? "Think"
      : entry.role === "tool"
        ? "Tool"
        : entry.role === "error"
          ? "Err"
          : entry.role === "status"
            ? "Stat"
            : entry.role === "notice"
              ? "Note"
              : "Asst"
  return `${label}: ${text.split("\n")[0]}`
}

export function previewLineForTerminalRecord(kind: TerminalOutputRecord["kind"], text: string) {
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
  if (!normalized) {
    return ""
  }
  const label = kind === "prompt_echo"
    ? "You"
    : kind === "provider_reasoning"
      ? "Think"
      : kind === "provider_tool"
        ? "Tool"
        : kind === "provider_error"
          ? "Err"
          : kind === "provider_status"
            ? "Stat"
            : "Asst"
  return `${label}: ${normalized.split("\n")[0]}`
}

export function computeCurrentTurnId(entries: TranscriptEntry[]) {
  return entries.reduce<number | null>((latest, entry) => {
    if (!entry || entry.role !== "user" || entry.turnId === undefined) {
      return latest
    }
    return entry.turnId
  }, null)
}

export function computeNextTurnId(entries: TranscriptEntry[]) {
  return entries.reduce((max, entry) => Math.max(max, entry?.turnId ?? 0), 0) + 1
}

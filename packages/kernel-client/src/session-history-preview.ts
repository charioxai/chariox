import type { SessionHistoryEntry, TranscriptEntry } from "./kernel-types.js"
import { externalProviderObservedProviderStatusShouldRender } from "./external-provider-observation.js"

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

export function sessionHistoryEntryPreviewLabel(kind: SessionHistoryEntry["kind"]): string {
  switch (kind) {
    case "user_prompt":
      return "You"
    case "provider_reasoning":
      return "Think"
    case "provider_tool":
      return "Tool"
    case "provider_error":
      return "Err"
    case "provider_status":
      return "Stat"
    case "notice":
      return "Note"
    case "provider_output":
      return "Asst"
  }
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

import type { TranscriptEntry } from "./cli-types.js"

export const TRANSCRIPT_ENTRY_PADDING = {
  horizontal: 1,
  vertical: 1,
} as const

export function transcriptEntryPadding(entry: TranscriptEntry) {
  if (entry.role === "turn_toggle") {
    return {
      horizontal: 0,
      vertical: 0,
    }
  }
  return TRANSCRIPT_ENTRY_PADDING
}

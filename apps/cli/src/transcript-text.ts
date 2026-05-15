import type { TranscriptEntry } from "./cli-types.js"

export function trimSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text
}

export function reindexTranscriptEntries(entries: TranscriptEntry[], startingId: number): TranscriptEntry[] {
  return entries.map((entry, index) => ({
    ...entry,
    id: startingId + index + 1,
  }))
}

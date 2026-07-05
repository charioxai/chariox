import type { TranscriptEntry } from "./cli-types.js"
import {
  reindexTranscriptEntries as sharedReindexTranscriptEntries,
  trimSingleTrailingNewline as sharedTrimSingleTrailingNewline,
} from "@arroba/kernel-client/transcript-entry-state"

export function trimSingleTrailingNewline(text: string): string {
  return sharedTrimSingleTrailingNewline(text)
}

export function reindexTranscriptEntries(entries: TranscriptEntry[], startingId: number): TranscriptEntry[] {
  return sharedReindexTranscriptEntries(entries, startingId)
}

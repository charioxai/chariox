import type {
  SessionHistoryPageEntry,
  TerminalOutputRecord,
  TranscriptEntry,
} from "./cli-types.js"
import {
  appendTranscriptPreviewLine,
  formatSessionHistoryPreview,
  formatTranscriptPreview as sharedFormatTranscriptPreview,
  previewLineForTerminalRecord as sharedPreviewLineForTerminalRecord,
} from "@arroba/kernel-client/session-history-preview"
import {
  computeCurrentTranscriptTurnId,
  computeNextTranscriptTurnId,
} from "@arroba/kernel-client/transcript-entry-state"

export function appendPreviewLine(current: string, line: string) {
  return appendTranscriptPreviewLine(current, line)
}

export function formatHistoryPreview(historyEntries: SessionHistoryPageEntry[]) {
  return formatSessionHistoryPreview(historyEntries)
}

export function formatTranscriptPreview(transcriptEntries: TranscriptEntry[]) {
  return sharedFormatTranscriptPreview(transcriptEntries)
}

export function previewLineForTerminalRecord(kind: TerminalOutputRecord["kind"], text: string) {
  return sharedPreviewLineForTerminalRecord(kind, text)
}

export function computeCurrentTurnId(entries: TranscriptEntry[]) {
  return computeCurrentTranscriptTurnId(entries)
}

export function computeNextTurnId(entries: TranscriptEntry[]) {
  return computeNextTranscriptTurnId(entries)
}

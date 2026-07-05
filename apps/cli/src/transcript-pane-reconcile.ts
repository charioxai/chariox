import {
  reconcileMountedTranscriptPane as sharedReconcileMountedTranscriptPane,
  transcriptEntriesEqual as sharedTranscriptEntriesEqual,
  transcriptEntriesShareMountedPrefix as sharedTranscriptEntriesShareMountedPrefix,
  type TranscriptPaneRenderable as SharedTranscriptPaneRenderable,
  type TranscriptPaneScrollbox,
} from "@arroba/kernel-client/transcript-pane-reconcile"
import type { TranscriptEntry } from "./cli-types.js"

export type TranscriptPaneRenderable = SharedTranscriptPaneRenderable<TranscriptEntry>
export type { TranscriptPaneScrollbox }

export function transcriptEntriesEqual(left: TranscriptEntry, right: TranscriptEntry) {
  return sharedTranscriptEntriesEqual(left, right)
}

export function transcriptEntriesShareMountedPrefix(left: TranscriptEntry, right: TranscriptEntry) {
  return sharedTranscriptEntriesShareMountedPrefix(left, right)
}

export function reconcileMountedTranscriptPane(options: {
  scrollbox: TranscriptPaneScrollbox | undefined
  currentEntries: TranscriptEntry[]
  nextEntries: TranscriptEntry[]
  renderables: Map<number, TranscriptPaneRenderable>
  clampScrollTop: (scrollTop: number, scrollHeight: number, viewportHeight: number) => number
  rebuild: () => void
  removeEmptyRenderable?: () => void
  mountEntry: (entry: TranscriptEntry, requestRender: boolean) => void
  onScrollTop?: (scrollTop: number) => void
}) {
  return sharedReconcileMountedTranscriptPane(options)
}

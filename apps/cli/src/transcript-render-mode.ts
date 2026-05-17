import type { TranscriptEntry } from "./cli-types.js"
import {
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  shouldRenderTranscriptAsMarkdown,
} from "./transcript.js"

export type TranscriptRenderMode =
  | "blob-collapsed"
  | "turn-toggle"
  | "patch"
  | "markdown"
  | "text"

export type TranscriptApplyPatchFiles = ReturnType<typeof readApplyPatchFiles>

export function transcriptRenderMode(entry: TranscriptEntry): TranscriptRenderMode {
  if (shouldRenderCollapsedTranscriptBlob(entry)) {
    return "blob-collapsed"
  }
  if (entry.role === "turn_toggle") {
    return "turn-toggle"
  }
  if (readTranscriptApplyPatch(entry)) {
    return "patch"
  }
  if (shouldRenderTranscriptAsMarkdown(entry.role, entry.text)) {
    return "markdown"
  }
  return "text"
}

export function shouldRenderCollapsedTranscriptBlob(entry: TranscriptEntry) {
  return entry.blobCollapsible === true && entry.blobCollapsed !== false
}

export function readTranscriptApplyPatch(entry: TranscriptEntry): TranscriptApplyPatchFiles | null {
  const parsed = parseToolTranscriptUpdate(entry.sourceText ?? entry.text)
  if (!parsed) {
    return null
  }
  const files = readApplyPatchFiles(parsed)
  return files.length > 0 ? files : null
}

import type { TranscriptEntry } from "./cli-types.js"

export type CollapsedTranscriptBlobPresentation = {
  headline: string
  detail: string
  actionLabel: string
  stateLabel: string
}

export function collapsedTranscriptBlobPresentation(entry: TranscriptEntry): CollapsedTranscriptBlobPresentation {
  const title = cleanText(entry.blobTitle) || roleBlobTitle(entry.role)
  const stateLabel = collapsedBlobStateLabel(entry)
  const summary = cleanText(entry.blobSummary)
  const heading = stateLabel ? `${title} · ${stateLabel}` : title

  return {
    headline: [`> ${heading}`, summary].filter(Boolean).join("  "),
    detail: collapsedBlobDetail(entry, stateLabel),
    actionLabel: collapsedBlobActionLabel(entry),
    stateLabel,
  }
}

export function roleBlobTitle(role: TranscriptEntry["role"]) {
  if (role === "turn_toggle") {
    return "turn"
  }
  return role
}

function collapsedBlobStateLabel(entry: TranscriptEntry) {
  if (entry.historyBlobError) {
    return "ERROR"
  }
  if (entry.historyBlobLoading) {
    return "LOADING"
  }
  if (entry.historyBlobLoaded) {
    return "LOADED"
  }
  if (entry.historyBlobId) {
    return "HISTORY"
  }
  return ""
}

function collapsedBlobDetail(entry: TranscriptEntry, stateLabel: string) {
  if (entry.historyBlobError) {
    return `History blob failed to load: ${entry.historyBlobError}`
  }
  if (entry.historyBlobLoading) {
    return "Loading history blob content"
  }
  if (entry.historyBlobId) {
    return "History blob content is collapsed"
  }
  if (stateLabel) {
    return `${stateLabel.toLowerCase()} blob content`
  }
  return "Collapsed blob content"
}

function collapsedBlobActionLabel(entry: TranscriptEntry) {
  if (entry.historyBlobLoading) {
    return "loading..."
  }
  if (entry.historyBlobError) {
    return "click to retry"
  }
  if (entry.historyBlobId) {
    return "click to load"
  }
  return "click to expand"
}

function cleanText(value: unknown) {
  return typeof value === "string" ? value.trim() : ""
}

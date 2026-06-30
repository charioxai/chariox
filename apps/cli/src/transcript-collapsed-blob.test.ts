import assert from "node:assert/strict"
import test from "node:test"

import { collapsedTranscriptBlobPresentation, roleBlobTitle } from "./transcript-collapsed-blob.js"
import type { TranscriptEntry } from "./cli-types.js"

test("collapsedTranscriptBlobPresentation formats normal collapsed blobs with a chevron header", () => {
  const presentation = collapsedTranscriptBlobPresentation({
    id: 1,
    role: "tool",
    text: "tool body",
    blobTitle: "bash · COMPLETED",
    blobSummary: "$ git status",
    blobCollapsible: true,
    blobCollapsed: true,
  })

  assert.deepEqual(presentation, {
    headline: "> bash · COMPLETED  $ git status",
    detail: "Collapsed blob content",
    actionLabel: "click to expand",
    stateLabel: "",
  })
})

test("collapsedTranscriptBlobPresentation labels unloaded history blobs distinctly", () => {
  const presentation = collapsedTranscriptBlobPresentation(historyBlobEntry())

  assert.deepEqual(presentation, {
    headline: "> tool · HISTORY  1 tool call",
    detail: "History blob content is collapsed",
    actionLabel: "click to load",
    stateLabel: "HISTORY",
  })
})

test("collapsedTranscriptBlobPresentation exposes history loading state", () => {
  const presentation = collapsedTranscriptBlobPresentation({
    ...historyBlobEntry(),
    historyBlobLoading: true,
    blobSummary: "loading...",
  })

  assert.deepEqual(presentation, {
    headline: "> tool · LOADING  loading...",
    detail: "Loading history blob content",
    actionLabel: "loading...",
    stateLabel: "LOADING",
  })
})

test("collapsedTranscriptBlobPresentation exposes history error state and retry action", () => {
  const presentation = collapsedTranscriptBlobPresentation({
    ...historyBlobEntry(),
    historyBlobLoading: false,
    historyBlobError: "network timeout",
    blobSummary: "failed: network timeout",
  })

  assert.deepEqual(presentation, {
    headline: "> tool · ERROR  failed: network timeout",
    detail: "History blob failed to load: network timeout",
    actionLabel: "click to retry",
    stateLabel: "ERROR",
  })
})

test("roleBlobTitle keeps collapsed role metadata specific", () => {
  assert.equal(roleBlobTitle("reasoning"), "reasoning")
  assert.equal(roleBlobTitle("status"), "status")
  assert.equal(roleBlobTitle("notice"), "notice")
})

function historyBlobEntry(): TranscriptEntry {
  return {
    id: 1,
    role: "tool",
    text: "",
    blobTitle: "tool",
    blobSummary: "1 tool call",
    blobCollapsible: true,
    blobCollapsed: true,
    historyBlobId: "blob-1",
    historyBlobAgentId: "agent-1",
    historyBlobLoaded: false,
  }
}

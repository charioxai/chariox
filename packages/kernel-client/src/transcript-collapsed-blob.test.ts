import assert from "node:assert/strict"
import test from "node:test"

import {
  collapsedTranscriptBlobPresentation,
  describeCollapsedTranscriptBlob,
  roleBlobTitle,
  type CollapsedTranscriptBlobEntry,
} from "./transcript-collapsed-blob.js"

test("collapsed transcript blob presentation formats normal collapsed blobs with a chevron header", () => {
  const presentation = collapsedTranscriptBlobPresentation({
    role: "tool",
    blobTitle: "bash · COMPLETED",
    blobSummary: "$ git status",
  })

  assert.deepEqual(presentation, {
    headline: "> bash · COMPLETED  $ git status",
    detail: "Collapsed blob content",
    actionLabel: "click to expand",
    stateLabel: "",
  })
})

test("collapsed transcript blob presentation labels unloaded history blobs distinctly", () => {
  const presentation = collapsedTranscriptBlobPresentation(historyBlobEntry())

  assert.deepEqual(presentation, {
    headline: "> tool · HISTORY  1 tool call",
    detail: "History blob content is collapsed",
    actionLabel: "click to load",
    stateLabel: "HISTORY",
  })
})

test("collapsed transcript blob presentation exposes history loading state", () => {
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

test("collapsed transcript blob presentation exposes history error state and retry action", () => {
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

test("describeCollapsedTranscriptBlob summarizes tool command metadata", () => {
  assert.deepEqual(describeCollapsedTranscriptBlob({
    role: "tool",
    text: "**bash** · COMPLETED\n\n**Command**\n```bash\n$ git status\n```",
    sourceText: JSON.stringify({
      id: "tool-1",
      tool: "bash",
      status: "completed",
      input: { command: "git status" },
    }),
  }), {
    title: "bash · COMPLETED",
    summary: "$ git status",
  })
})

test("describeCollapsedTranscriptBlob summarizes tool-specific structured metadata", () => {
  assert.deepEqual(describeCollapsedTranscriptBlob({
    role: "tool",
    sourceText: JSON.stringify({
      id: "tool-read",
      tool: "read",
      status: "running",
      input: { path: "src/app.ts", offset: 5, limit: 20 },
    }),
  }), {
    title: "read · RUNNING",
    summary: "src/app.ts [offset=5, limit=20]",
  })

  assert.deepEqual(describeCollapsedTranscriptBlob({
    role: "tool",
    sourceText: JSON.stringify({
      id: "tool-todos",
      tool: "todowrite",
      status: "completed",
      input: {
        todos: [
          { status: "completed" },
          { status: "in_progress" },
          { status: "pending" },
        ],
      },
    }),
  }), {
    title: "todowrite · COMPLETED",
    summary: "2 remaining of 3",
  })
})

function historyBlobEntry(): CollapsedTranscriptBlobEntry {
  return {
    role: "tool",
    blobTitle: "tool",
    blobSummary: "1 tool call",
    historyBlobId: "blob-1",
    historyBlobLoaded: false,
  }
}

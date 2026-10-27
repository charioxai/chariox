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
    title: "Ran git status · COMPLETED",
    icon: "terminal",
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
    title: "Reading src/app.ts · RUNNING",
    icon: "read",
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
    title: "Updated plan · COMPLETED",
    icon: "plan",
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

// MP-08 / MP-10 (panes R15, PANE-07): thoughts are titled "Thought" with their
// first line, never the raw role name.
test("describeCollapsedTranscriptBlob titles a thought with its first line", () => {
  assert.deepEqual(describeCollapsedTranscriptBlob({
    role: "reasoning",
    text: "**Checking the README**\nThe file lists three steps.",
  }), {
    title: "Thought",
    summary: "Checking the README",
  })
})

// MP-08 / MP-10 (panes, Miguel 2026-10-12 addition A): a tool row's headline
// shows its kind's single-width glyph left of the title.
test("collapsed tool blob headline shows the tool kind glyph before the title", () => {
  const sourceText = JSON.stringify({ id: "call-1", tool: "exec_command", status: "completed", input: { cmd: "pnpm test" } })
  const described = describeCollapsedTranscriptBlob({ role: "tool", text: "", sourceText })
  assert.equal(described.icon, "terminal")
  const presentation = collapsedTranscriptBlobPresentation({ role: "tool", blobTitle: described.title, blobSummary: described.summary, blobIcon: described.icon })
  assert.match(presentation.headline, /^> \$ Ran pnpm test/)
  assert.match(collapsedTranscriptBlobPresentation({ role: "reasoning", blobTitle: "Thought", blobSummary: "plan" }).headline, /^> Thought/)
})

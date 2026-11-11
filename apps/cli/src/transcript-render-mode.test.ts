import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  readTranscriptApplyPatch,
  shouldRenderCollapsedTranscriptBlob,
  transcriptRenderMode,
} from "./transcript-render-mode.js"

function transcriptEntry(role: TranscriptEntry["role"], overrides: Partial<TranscriptEntry> = {}): TranscriptEntry {
  return {
    id: 1,
    role,
    text: "hello",
    ...overrides,
  } as TranscriptEntry
}

function applyPatchEntry(): TranscriptEntry {
  return transcriptEntry("tool", {
    text: "apply_patch",
    sourceText: JSON.stringify({
      id: "tool-1",
      tool: "apply_patch",
      input: {
        patchText: [
          "*** Begin Patch",
          "*** Update File: src/app.ts",
          "@@",
          "-const oldValue = 1",
          "+const newValue = 2",
          "*** End Patch",
        ].join("\n"),
      },
    }),
  })
}

test("transcript render mode prioritizes collapsed blobs and turn toggles", () => {
  assert.equal(
    transcriptRenderMode(transcriptEntry("user", { blobCollapsible: true })),
    "blob-collapsed",
  )
  assert.equal(
    transcriptRenderMode(transcriptEntry("user", { blobCollapsible: true, blobCollapsed: false })),
    "text",
  )
  assert.equal(transcriptRenderMode(transcriptEntry("turn_toggle")), "turn-toggle")
})

test("transcript render mode detects patches and markdown before plain text", () => {
  assert.equal(transcriptRenderMode(applyPatchEntry()), "patch")
  assert.equal(
    transcriptRenderMode(transcriptEntry("assistant", { text: "```ts\nconst value = 1\n```" })),
    "markdown",
  )
  assert.equal(transcriptRenderMode(transcriptEntry("user", { text: "plain prompt" })), "text")
})

test("readTranscriptApplyPatch extracts patch files from tool source text", () => {
  const files = readTranscriptApplyPatch(applyPatchEntry())

  assert.equal(shouldRenderCollapsedTranscriptBlob(transcriptEntry("tool", { blobCollapsible: true })), true)
  assert.equal(files?.length, 1)
  assert.equal(files?.[0]?.filePath, "src/app.ts")
})

test("MP-08 a paragraph ending in a visualize line renders as markdown so the page is named", () => {
  assert.equal(
    transcriptRenderMode(transcriptEntry("assistant", { text: 'Here is the chart.\n\nvisualize{"path":"/w/chart.html"}' })),
    "markdown",
  )
  assert.equal(transcriptRenderMode(transcriptEntry("assistant", { text: 'visualize{"path": 3}' })), "text", "not a page: plain text")
})

test("MP-08 a reply ending in Codex's wrapped visualize line renders as markdown so the page is named", () => {
  assert.equal(
    transcriptRenderMode(transcriptEntry("assistant", { text: 'Here is the chart.\n\nvisualize{"path":"/w/chart.html"}' })),
    "markdown",
  )
})

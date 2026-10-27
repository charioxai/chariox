import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  createTranscriptRetentionController,
  type TranscriptRenderableHandle,
} from "./transcript-retention-controller.js"

function transcriptEntry(id: number, text: string, mergeKey?: string): TranscriptEntry {
  return {
    id,
    role: "assistant",
    text,
    ...(mergeKey ? { mergeKey } : {}),
  } as TranscriptEntry
}

test("transcript retention trims old entries by count and cleans mounted renderables", () => {
  let entries = [
    transcriptEntry(1, "one", "tool-1"),
    transcriptEntry(2, "two"),
    transcriptEntry(3, "three"),
  ]
  const destroyed: string[] = []
  const removed: string[] = []
  const deletedTools: string[] = []
  const renderables = new Map<number, TranscriptRenderableHandle>([
    [1, renderable("renderable-1", destroyed)],
    [2, renderable("renderable-2", destroyed)],
  ])

  const controller = createTranscriptRetentionController({
    entries: () => entries,
    setEntries: (nextEntries) => {
      entries = nextEntries
    },
    renderables,
    removeFromScrollbox: (renderableId) => {
      removed.push(renderableId)
      return true
    },
    requestScrollboxRender: () => {
      removed.push("render")
    },
    deleteTool: (mergeKey) => {
      deletedTools.push(mergeKey)
    },
    maxEntries: 2,
    maxChars: 1_000,
  })

  controller.enforce()

  assert.deepEqual(entries.map((entry) => entry.id), [2, 3])
  assert.deepEqual(removed, ["renderable-1", "render"])
  assert.deepEqual(destroyed, ["renderable-1"])
  assert.deepEqual(deletedTools, ["tool-1"])
  assert.equal(renderables.has(1), false)
})

test("transcript retention keeps at least one entry when trimming by characters", () => {
  let entries = [
    transcriptEntry(1, "older"),
    transcriptEntry(2, "large-current-entry"),
  ]
  const controller = createTranscriptRetentionController({
    entries: () => entries,
    setEntries: (nextEntries) => {
      entries = nextEntries
    },
    renderables: new Map(),
    removeFromScrollbox: () => true,
    requestScrollboxRender: () => {},
    deleteTool: () => {},
    maxEntries: 10,
    maxChars: 4,
  })

  controller.enforce()

  assert.deepEqual(entries.map((entry) => entry.id), [2])
})

function renderable(id: string, destroyed: string[]): TranscriptRenderableHandle {
  return {
    wrapper: {
      id,
      destroyRecursively: () => {
        destroyed.push(id)
      },
    },
  }
}

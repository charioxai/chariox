import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { TranscriptEntry } from "./cli-types.js"
import { buildCollapsedTranscriptBlob } from "./transcript-interaction-render.js"

test("MP-08 / MP-10 native collapsed tool headline and action each toggle exactly one row", async () => {
  const h = await createTestRenderer({ width: 80, height: 10, useThread: false })
  const body = new BoxRenderable(h.renderer, { width: "100%", flexDirection: "column" })
  const toggled: unknown[] = []
  h.renderer.root.add(body)
  buildCollapsedTranscriptBlob(h.renderer, body, {
    id: 7, role: "tool", text: "Read README.md", blobTitle: "Read README.md", blobSummary: "source file", blobCollapsible: true, blobCollapsed: true,
  } as TranscriptEntry, (id, collapsed) => toggled.push([id, collapsed]))
  try {
    await h.renderOnce()
    const lines = h.captureCharFrame().split("\n")
    const headline = lines.findIndex(line => line.includes("Read README.md"))
    const action = lines.findIndex(line => line.includes("click to"))
    assert.ok(headline >= 0 && action > headline)
    await h.mockMouse.click(3, headline, 0)
    await new Promise(resolve => setTimeout(resolve, 20))
    assert.deepEqual(toggled, [[7, false]])
    await h.mockMouse.click(3, action, 0)
    await new Promise(resolve => setTimeout(resolve, 20))
    assert.deepEqual(toggled, [[7, false], [7, false]])
    await h.mockMouse.click(3, headline, 2)
    await new Promise(resolve => setTimeout(resolve, 20))
    assert.equal(toggled.length, 2)
  } finally { h.renderer.destroy() }
})

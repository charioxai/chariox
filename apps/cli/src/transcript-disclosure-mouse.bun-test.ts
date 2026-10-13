import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable, ScrollBoxRenderable, TextRenderable } from "@opentui/core"
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

test("MP-08 / MP-10 a scrolled native tool headline remains clickable", async () => {
  const h = await createTestRenderer({ width: 80, height: 8, useThread: false })
  const scroll = new ScrollBoxRenderable(h.renderer, { width: 80, height: 8, stickyScroll: true, stickyStart: "bottom" })
  h.renderer.root.add(scroll)
  scroll.add(new TextRenderable(h.renderer, { content: "source line\n".repeat(50) }))
  const body = new BoxRenderable(h.renderer, { width: "100%", flexDirection: "column" })
  scroll.add(body)
  const toggled: number[] = []
  buildCollapsedTranscriptBlob(h.renderer, body, { id: 7, role: "tool", text: "Read README.md", blobTitle: "Read README.md", blobCollapsible: true, blobCollapsed: true } as TranscriptEntry, id=>toggled.push(id))
  try {
    await h.renderOnce()
    scroll.scrollTo(scroll.scrollHeight)
    await h.renderOnce()
    const row = h.captureCharFrame().split("\n").findIndex(line=>line.includes("Read README.md"))
    assert.ok(row >= 0, h.captureCharFrame())
    await h.mockMouse.click(3, row, 0)
    await new Promise(resolve=>setTimeout(resolve,20))
    assert.deepEqual(toggled,[7])
  } finally {h.renderer.destroy()}
})

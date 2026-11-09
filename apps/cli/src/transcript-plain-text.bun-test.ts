import assert from "node:assert/strict"
import test from "node:test"
import { createTestRenderer } from "@opentui/core/testing"
import { buildTranscriptEntryRenderable } from "./transcript-render.js"
import { createTranscriptSyntaxStyle } from "./theme.js"
import type { TranscriptEntry } from "./cli-types.js"

test("MP-08 / MP-10 plain assistant text paints exact identifiers on its first frame", async () => {
  const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
  const syntax = createTranscriptSyntaxStyle()
  const entry: TranscriptEntry = { id: 1, role: "assistant", text: "ready_value_2 001\nready_value_2 002" }
  const node = buildTranscriptEntryRenderable(harness.renderer, entry, syntax, () => {}, () => {})
  harness.renderer.root.add(node.wrapper)
  try {
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("ready_value_2 001"), "plain text must not wait for syntax highlighting")
    node.update({ ...entry, text: entry.text + "\nready_value_2 003" })
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("ready_value_2 003"))
    node.update({ ...entry, text: "```text\nready_value_2 004\n```" })
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("ready_value_2 004"), "a later code fence retains its exact contents")
  } finally { harness.renderer.destroy(); syntax.destroy() }
})

test("MP-08 a reply ending in a visualize line names the page, while a fenced one stays code", async () => {
  const harness = await createTestRenderer({ width: 100, height: 8, useThread: false })
  const syntax = createTranscriptSyntaxStyle()
  const text = 'Here is the chart.\n\nvisualize{"path":"/w/chart.html"}'
  const entry: TranscriptEntry = { id: 1, role: "assistant", text }
  const node = buildTranscriptEntryRenderable(harness.renderer, entry, syntax, () => {}, () => {})
  harness.renderer.root.add(node.wrapper)
  // Markdown paints after the syntax worker answers, so wait for the settled frame.
  const settledFrame = async (painted: (frame: string) => boolean) => {
    for (let attempt = 0; attempt < 40; attempt += 1) {
      await harness.renderOnce()
      if (painted(harness.captureCharFrame())) break
      await new Promise((resolve) => setTimeout(resolve, 25))
    }
    return harness.captureCharFrame()
  }
  try {
    let frame = await settledFrame((frame) => frame.includes("Here is the chart."))
    assert.ok(frame.includes("▣ visual: chart.html · /w/chart.html (open in the web app)"), frame)
    assert.ok(!frame.includes("visualize{"), frame)
    node.update({ ...entry, text: "Here is the chart." })
    node.update({ ...entry, text })
    frame = await settledFrame((frame) => frame.includes("▣ visual"))
    assert.ok(frame.includes("▣ visual: chart.html"), "a streamed visualize line is named too")
    node.update({ ...entry, text: '```\nvisualize{"path":"/w/chart.html"}\n```' })
    frame = await settledFrame((frame) => frame.includes("visualize{"))
    assert.ok(frame.includes('visualize{"path":"/w/chart.html"}') && !frame.includes("▣ visual"), frame)
  } finally { harness.renderer.destroy(); syntax.destroy() }
})

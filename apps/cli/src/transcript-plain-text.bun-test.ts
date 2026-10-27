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

import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as delay } from "node:timers/promises"
import { createTestRenderer } from "@opentui/core/testing"
import type { TranscriptEntry } from "./cli-types.js"
import { createTranscriptSyntaxStyle } from "./theme.js"
import { buildTranscriptEntryRenderable } from "./transcript-render.js"

test("the expanded terminal transcript renders compaction states as readable text", async () => {
  const harness = await createTestRenderer({ width: 100, height: 14, useThread: false })
  const syntax = createTranscriptSyntaxStyle()
  const row = (state: object): TranscriptEntry => ({ id: 1, role: "compaction", text: JSON.stringify(state) })
  const rendered = buildTranscriptEntryRenderable(
    harness.renderer, row({ state: "running", started_at_ms: Date.now() - 5000 }), syntax, () => {}, () => {},
  )
  harness.renderer.root.add(rendered.wrapper)
  try {
    await harness.renderOnce()
    const running = /Compacting context… · (\d+)s/
    const elapsed = Number(harness.captureCharFrame().match(running)?.[1])
    assert.ok(elapsed >= 5)
    await delay(1100)
    await harness.renderOnce()
    assert.ok(Number(harness.captureCharFrame().match(running)?.[1]) > elapsed)
    for (const [state, label, detail] of [
      [{ state: "done", summary: "Saved context" }, "Context compacted", "Saved context"],
      [{ state: "done", summary: "" }, "Context compacted", "Summary unavailable"],
      [{ state: "failed", reason: "The turn was cancelled." }, "Context compaction failed", "The turn was cancelled."],
    ] as const) {
      rendered.update(row(state))
      await harness.renderOnce()
      const renderedText = harness.captureCharFrame()
      assert.ok(renderedText.includes(label))
      assert.ok(renderedText.includes(detail))
      assert.doesNotMatch(renderedText, /"state":/)
    }
  } finally {
    harness.renderer.destroy()
    syntax.destroy()
  }
})

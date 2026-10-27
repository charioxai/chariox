import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createSessionChromeSummaryRenderState, renderSessionChromeSummary } from "./session-chrome-summary-renderer.js"

test("MP-08 / MP-11 a late chrome update cannot write destroyed terminal buffers", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const promptStateBox = new BoxRenderable(harness.renderer, {})
  const footerSummaryBox = new BoxRenderable(harness.renderer, {})
  harness.renderer.root.add(promptStateBox)
  harness.renderer.root.add(footerSummaryBox)
  const options = {
    renderer: harness.renderer, state: createSessionChromeSummaryRenderState(),
    promptStateBox, footerSummaryBox, promptStateLabel: "IDLE",
    promptStateTone: "muted" as const, footerSummary: "Session loginux", footerFlash: null,
  }
  try {
    renderSessionChromeSummary(options)
    harness.renderer.destroy()
    assert.doesNotThrow(() => renderSessionChromeSummary(options))
  } finally {
    if (!harness.renderer.isDestroyed) harness.renderer.destroy()
  }
})

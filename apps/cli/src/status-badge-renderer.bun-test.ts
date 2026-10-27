import assert from "node:assert/strict"
import test from "node:test"
import type { TextRenderable } from "@opentui/core"
import { renderStatusBadgeLabel } from "./status-badge-renderer.js"

function fakeTexts(count: number) {
  const writes: string[] = []
  const texts = Array.from({ length: count }, () => {
    let content = " ", fg: unknown, attributes = 0
    return {
      get content() { return content }, set content(value: string) { content = value; writes.push("content") },
      get fg() { return fg }, set fg(value: unknown) { fg = value; writes.push("fg") },
      get attributes() { return attributes }, set attributes(value: number) { attributes = value; writes.push("attributes") },
    }
  })
  return { texts: texts as unknown as TextRenderable[], writes }
}

// MP-08 / MP-10 PANE-03: each assignment rebuilds the native text buffer, and the
// startup repaints the same badge many times.
test("MP-08 / MP-10 PANE-03 repainting an unchanged status badge does not rewrite its text cells", () => {
  const { texts, writes } = fakeTexts(8)
  renderStatusBadgeLabel(texts, "IDLE", "idle", 8, 0)
  assert.equal(texts.map((text) => text.content).join(""), "IDLE    ")
  const first = writes.length
  renderStatusBadgeLabel(texts, "IDLE", "idle", 8, 3)
  renderStatusBadgeLabel(texts, "IDLE", "idle", 8, 4)
  assert.equal(writes.length, first, "an identical badge must not touch the native buffer again")
  renderStatusBadgeLabel(texts, "WORKING", "working", 8, 1)
  assert.equal(texts.map((text) => text.content).join(""), "WORKING ")
  const working = writes.length
  renderStatusBadgeLabel(texts, "WORKING", "working", 8, 2)
  assert.ok(writes.length > working, "the working animation still moves its highlight")
})

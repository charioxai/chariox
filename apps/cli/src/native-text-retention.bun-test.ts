import assert from "node:assert/strict"
import test from "node:test"
import { setImmediate as nextTick } from "node:timers/promises"
import { StyledText, SyntaxStyle, TextBuffer, TextRenderable, resolveRenderLib } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"

// The native text-buffer arena is absent from the JS heap and from OpenTUI's
// global EditorView arena counter. Track its actual backing allocations.
test("empty styled/plain/clear updates keep native rope allocations bounded", async () => {
  for (const method of ["styled", "plain", "clear"] as const) {
    const harness = await createTestRenderer({ width: 120, height: 40 })
    const text = new TextRenderable(harness.renderer, { content: "", wrapMode: "word" })
    harness.renderer.root.add(text)
    const lib = resolveRenderLib()
    const buffer = method === "styled" ? null : TextBuffer.create("unicode")
    const plainText = () => buffer ? buffer.getPlainText() : text.plainText
    const setText = (value: string) => {
      if (buffer) buffer.setText(value)
      else text.content = value
    }
    const update = () => {
      if (method === "styled") text.content = ""
      else if (method === "plain") buffer!.setText("")
      else buffer!.clear()
    }
    try {
      for (let i = 0; i < 10_000; i += 1) update()
      await harness.renderOnce()
      const before = lib.getAllocatorStats().largeAllocations
      for (let i = 0; i < 100_000; i += 1) {
        update()
        if (i % 1_000 === 0) await nextTick()
      }
      await harness.renderOnce()
      assert.ok(lib.getAllocatorStats().largeAllocations <= before + 1,
        `${method} updates retained native rope arena allocations`)
      assert.equal(plainText(), "")
      setText("first nonempty replacement")
      await harness.renderOnce()
      assert.equal(plainText(), "first nonempty replacement")
      if (method === "styled") assert.match(harness.captureCharFrame(), /first nonempty replacement/)
      update()
      await harness.renderOnce()
      assert.equal(plainText(), "")
      if (method === "styled") assert.doesNotMatch(harness.captureCharFrame(), /first nonempty replacement/)
      setText("another replacement")
      await harness.renderOnce()
      assert.equal(plainText(), "another replacement")
      if (method === "styled") assert.match(harness.captureCharFrame(), /another replacement/)
    } finally {
      buffer?.destroy()
      text.destroyRecursively()
      harness.renderer.destroy()
    }
  }
})

test("empty styled replacements preserve highlight-clearing behavior", () => {
  const buffer = TextBuffer.create("unicode")
  const style = SyntaxStyle.create()
  try {
    const styleId = style.registerStyle("marker", { bold: true })
    buffer.setSyntaxStyle(style)
    buffer.setText("abc")
    buffer.addHighlightByCharRange({ start: 0, end: 2, styleId })
    assert.equal(buffer.getHighlightCount(), 1)
    buffer.clear()
    // The published empty-list path only clears text; an empty styled chunk
    // also clears highlights, even when the text buffer is already empty.
    buffer.setStyledText(new StyledText([]))
    assert.equal(buffer.getHighlightCount(), 1)
    buffer.setStyledText(new StyledText([{ __isChunk: true, text: "" }]))
    assert.equal(buffer.getHighlightCount(), 0)
  } finally {
    buffer.destroy()
    style.destroy()
  }
})

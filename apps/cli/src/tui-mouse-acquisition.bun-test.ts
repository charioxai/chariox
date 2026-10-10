import assert from "node:assert/strict"
import test from "node:test"
import { StdinParser, TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createClipboardController } from "./clipboard-controller.js"

// MP-08 / MP-10: SSH reads and a busy renderer may deliver a mouse report
// after the 10 ms lone-Escape deadline. Exercise the actual renderer/parser.
for (const stage of ["down", "drag", "slow-drag"] as const) for (const split of [1, 7]) {
  test(`MP-08 / MP-10 delayed ${stage} report at byte ${split} acquires selection`, async () => {
    const harness = await createTestRenderer({ width: 50, height: 5, useThread: false })
    const text = new TextRenderable(harness.renderer, { content: "TUIFIX MARKER SEVEN", width: 40, height: 1 })
    harness.renderer.root.add(text)
    await harness.renderOnce()
    const clipboard = createClipboardController({ renderer: harness.renderer, promptInput: () => null, flashFooter: () => {} })
    harness.renderer.keyInput.prependListener("keypress", event => clipboard.captureCopyKey(event))
    const send = (bytes: string) => harness.renderer.stdin.emit("data", Buffer.from(bytes))
    const down = "\x1b[<0;1;1M", drag = "\x1b[<32;14;1M"
    try {
      if (stage !== "down") send(down)
      const report = stage === "down" ? down : drag
      send(report.slice(0, split))
      await new Promise(resolve => setTimeout(resolve, stage === "slow-drag" ? 400 : 50))
      send(report.slice(split))
      if (stage === "down") send(drag)
      send("\x1b[<0;14;1m")
      assert.equal(harness.renderer.getSelection()?.getSelectedText(), "TUIFIX MARKER")
    } finally { harness.renderer.destroy() }
  })
}

test("MP-08 / MP-10 known CSI mouse framing survives a long pause and resumes keys", () => {
  let now = 0
  const parser = new StdinParser({ timeoutMs: 10, armTimeouts: false, clock: { now: () => now, setTimeout, clearTimeout } })
  const events: Array<{ type: string; key?: { name: string } }> = []
  parser.push(Buffer.from("\x1b[<0;1;"))
  now = 1000
  parser.flushTimeout(now)
  parser.drain(event => events.push(event))
  assert.equal(events.length, 0, "a partial mouse report must never become text")
  parser.push(Buffer.from("1M\x1b[<32;14;1M\x1b[<0;14;1mx"))
  parser.drain(event => events.push(event))
  assert.deepEqual(events.map(event => event.type), ["mouse", "mouse", "mouse", "key"])
  assert.equal(events[3]?.key?.name, "x")
  parser.destroy()
})

import assert from "node:assert/strict"
import test from "node:test"
import { StdinParser, TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"

const down = "\x1b[<0;1;1M", drag = "\x1b[<32;14;1M", up = "\x1b[<0;14;1m"

// MP-08 / MP-10: SSH reads and a busy renderer may deliver the rest of a mouse
// report after the 10 ms parser deadline. Exercise the actual renderer/parser.
for (const [name, first, report, pause] of [
  ["down", "", down, 50],
  ["drag", down, drag, 50],
  ["slow-drag", down, drag, 400],
] as const) for (const split of name === "down" ? [7] : [1, 7]) {
  test(`MP-08 / MP-10 delayed ${name} report at byte ${split} acquires selection`, async () => {
    const harness = await createTestRenderer({ width: 50, height: 5, useThread: false })
    harness.renderer.root.add(new TextRenderable(harness.renderer, { content: "TUIFIX MARKER SEVEN", width: 40, height: 1 }))
    await harness.renderOnce()
    const send = (bytes: string) => harness.renderer.stdin.emit("data", Buffer.from(bytes))
    try {
      if (first) send(first)
      send(report.slice(0, split))
      await new Promise(resolve => setTimeout(resolve, pause))
      send(report.slice(split))
      if (name === "down") send(drag)
      send(up)
      assert.equal(harness.renderer.getSelection()?.getSelectedText(), "TUIFIX MARKER")
    } finally { harness.renderer.destroy() }
  })
}

test("MP-08 / MP-10 known CSI framing survives a long pause and resumes keys", () => {
  let now = 0
  const parser = new StdinParser({ timeoutMs: 10, armTimeouts: false, clock: { now: () => now, setTimeout, clearTimeout } })
  const events: Array<{ type: string; key?: { name: string } }> = []
  parser.push(Buffer.from("\x1b[<0;1;"))
  now = 1000
  parser.flushTimeout(now)
  parser.drain(event => events.push(event))
  assert.equal(events.length, 0, "a partial mouse report must never become text")
  parser.push(Buffer.from(`1M${drag}${up}x`))
  parser.drain(event => events.push(event))
  assert.deepEqual(events.map(event => event.type), ["mouse", "mouse", "mouse", "key"])
  assert.equal(events[3]?.key?.name, "x")
  parser.destroy()
})

test("MP-08 / MP-10 an incomplete CSI is abandoned when a new escape sequence starts", () => {
  const parser = new StdinParser({ timeoutMs: 10, armTimeouts: false })
  const names: string[] = []
  parser.push(Buffer.from("\x1b[<0;1;\x1b[A"))
  parser.drain(event => { if (event.type === "key" && event.key) names.push(event.key.name) })
  assert.ok(names.includes("up"), `the arrow key after an abandoned report must decode, got ${JSON.stringify(names)}`)
  parser.destroy()
})

// A release the terminal never delivered (drag left the window) must not make
// Escape wait for ever: after the bounded hold the Escape is its own key, the
// stale button state is dropped and the next key is not an Alt chord.
const missedReleaseHoldMs = 500

test("MP-08 / MP-10 a missed mouse release does not turn Escape plus a key into an Alt chord", () => {
  let now = 0
  const parser = new StdinParser({ timeoutMs: 10, armTimeouts: false, clock: { now: () => now, setTimeout, clearTimeout } })
  const keys: Array<{ name: string; meta: boolean }> = []
  const drain = () => parser.drain(event => { if (event.type === "key" && event.key) keys.push({ name: event.key.name, meta: event.key.meta }) })
  parser.push(Buffer.from(down))
  parser.push(Buffer.from("\x1b"))
  now = 100
  parser.flushTimeout(now)
  drain()
  assert.deepEqual(keys, [], "a pending Escape still waits briefly for a delayed report while a button is down")
  now = missedReleaseHoldMs + 100
  parser.flushTimeout(now)
  drain()
  assert.deepEqual(keys, [{ name: "escape", meta: false }])
  parser.push(Buffer.from("c"))
  drain()
  assert.deepEqual(keys.slice(1), [{ name: "c", meta: false }])
  parser.push(Buffer.from("\x1b"))
  now += 20
  parser.flushTimeout(now)
  drain()
  assert.deepEqual(keys.slice(2), [{ name: "escape", meta: false }], "the next lone Escape is back on the 10 ms deadline")
  parser.destroy()
})

test("MP-08 / MP-10 the armed timer recovers Escape after a missed mouse release", async () => {
  let flushed!: () => void
  const flushedOnce = new Promise<void>(resolve => { flushed = resolve })
  const parser = new StdinParser({ timeoutMs: 10, armTimeouts: true, onTimeoutFlush: () => flushed() })
  const keys: Array<{ name: string; meta: boolean }> = []
  const drain = () => parser.drain(event => { if (event.type === "key" && event.key) keys.push({ name: event.key.name, meta: event.key.meta }) })
  parser.push(Buffer.from(down))
  parser.push(Buffer.from("\x1b"))
  await flushedOnce
  drain()
  parser.push(Buffer.from("c"))
  drain()
  assert.deepEqual(keys, [{ name: "escape", meta: false }, { name: "c", meta: false }])
  parser.destroy()
})

test("MP-08 / MP-10 the renderer keeps Escape and the next key apart after a missed mouse release", async () => {
  const harness = await createTestRenderer({ width: 50, height: 5, useThread: false })
  const keys: Array<{ name: string; meta: boolean }> = []
  harness.renderer.keyInput.on("keypress", key => keys.push({ name: key.name, meta: key.meta }))
  const send = (bytes: string) => harness.renderer.stdin.emit("data", Buffer.from(bytes))
  try {
    send(down)
    send("\x1b")
    await new Promise(resolve => setTimeout(resolve, missedReleaseHoldMs + 200))
    send("c")
    assert.deepEqual(keys, [{ name: "escape", meta: false }, { name: "c", meta: false }])
  } finally { harness.renderer.destroy() }
})

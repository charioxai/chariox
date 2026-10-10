import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { closeSync, mkdtempSync, openSync, readFileSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { createSelectionTextView, selectionTextViewContent } from "./selection-text-view.js"

test("MP-08 / MP-10 / MP-11 plain selection retains logical lines and strips terminal instructions", () => {
  const paragraph = "one soft-wrapped paragraph ".repeat(12)
  assert.equal(selectionTextViewContent(paragraph + "\nnext λ"), paragraph + "\r\nnext λ")
  assert.equal(selectionTextViewContent("\x1b[31mred\x1b[0m\x1b]52;c;ZXZpbA==\x07\nnext\x00\x08\x9b"), "red\r\nnext")
})

test("MP-08 / MP-10 clean F7 view suspends layout, prints only selected logical text, then restores it", async () => {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "selection-view-"))
  const output = path.join(directory, "terminal")
  const fd = openSync(output, "w")
  const input = Object.assign(new EventEmitter(), { isTTY: true, isRaw: true, setRawMode: () => {}, resume: () => {} })
  const calls: string[] = []
  const renderer = { suspend: () => { calls.push("suspend") }, resume: () => { calls.push("resume") }, idle: async () => {}, clearSelection: () => { calls.push("clear") } }
  const errors: unknown[] = []
  const view = createSelectionTextView(renderer, error => errors.push(error), { input, output: { isTTY: true, fd } })
  const text = "A long paragraph ".repeat(30) + "\nA second logical line."
  try {
    assert.equal(view.present(text), true)
    assert.equal(view.isActive(), true)
    assert.equal(view.present("other"), false)
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(readFileSync(output, "utf8").split("\r\n\r\n")[1], text.replace(/\n/g, "\r\n") + "\r\n")
    input.emit("data", Buffer.from("\x1b[18~"))
    await new Promise(resolve => setImmediate(resolve))
    assert.deepEqual(calls, ["suspend", "clear", "resume"])
    assert.equal(view.isActive(), false)
    assert.equal(input.listenerCount("data"), 0)
    assert.deepEqual(errors, [])
    const url = "https://example.org/authorize?state=" + "abcdef".repeat(60)
    assert.equal(view.present(url), true)
    await new Promise(resolve => setImmediate(resolve))
    assert.ok(readFileSync(output, "utf8").includes(`\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\\r\n`))
    view.cancel()
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(view.present(text), true)
    view.cancel() // Cancellation before idle completes must not install a listener.
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(view.isActive(), false)
    assert.equal(input.listenerCount("data"), 0)
  } finally { view.cancel(); closeSync(fd); rmSync(directory, { recursive: true, force: true }) }
})

// MP-08 / MP-10: stdin is a stream, including while the renderer is suspended.
async function withTextView(run: (view: ReturnType<typeof createSelectionTextView>, input: EventEmitter, calls: string[]) => Promise<void>) {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "selection-stream-"))
  const fd = openSync(path.join(directory, "terminal"), "w")
  const input = Object.assign(new EventEmitter(), { isTTY: true, isRaw: true, setRawMode: () => {}, resume: () => {} })
  const calls: string[] = []
  const errors: unknown[] = []
  const view = createSelectionTextView({ suspend: () => {}, idle: async () => {}, clearSelection: () => {}, resume: () => { calls.push("resume") } }, error => errors.push(error), { input, output: { isTTY: true, fd } })
  try { await run(view, input, calls); assert.deepEqual(errors, []) }
  finally { view.cancel(); await tick(); closeSync(fd); rmSync(directory, { recursive: true, force: true }) }
}
const tick = () => new Promise<void>(resolve => setImmediate(resolve))
const delay = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms))

for (const buffers of [false, true]) {
  const chunk = (text: string) => buffers ? Buffer.from(text) : text
  test(`MP-08 / MP-10 F7 split at every boundary (${buffers ? "Buffer" : "string"})`, async () => {
    await withTextView(async (view, input, calls) => {
      const key = "\x1b[18~"
      for (let split = 1; split < key.length; split++) {
        assert.equal(view.present("selected"), true)
        await tick()
        input.emit("data", chunk(key.slice(0, split)))
        await tick()
        assert.equal(view.isActive(), true, `fragment at ${split} must not be standalone Escape`)
        input.emit("data", chunk(key.slice(split)))
        await tick()
        assert.equal(view.isActive(), false, `F7 boundary ${split} must return`)
        assert.equal(calls.length, split)
        assert.equal(input.listenerCount("data"), 0)
      }
    })
  })
  test(`MP-08 / MP-10 coalesced return keys consume remaining view input (${buffers ? "Buffer" : "string"})`, async () => {
    await withTextView(async (view, input, calls) => {
      for (const key of ["\r", "\n", "\x03", "\x1b[18~"]) {
        assert.equal(view.present("selected"), true)
        await tick()
        input.emit("data", chunk("before" + key + "z\x1b["))
        await tick()
        assert.equal(view.isActive(), false, `coalesced ${JSON.stringify(key)} must return`)
        assert.equal(input.listenerCount("data"), 0)
        const returned = calls.length
        // A pending trailing escape prefix must not close a later presentation.
        assert.equal(view.present("next"), true)
        await tick(); await delay(320)
        assert.equal(view.isActive(), true)
        assert.equal(calls.length, returned)
        view.cancel(); await tick()
      }
    })
  })
  test(`MP-08 / MP-10 unrelated fragmented escape and pasted return keys stay in the view (${buffers ? "Buffer" : "string"})`, async () => {
    await withTextView(async (view, input) => {
      assert.equal(view.present("selected"), true)
      await tick()
      for (const part of ["\x1b", "[1;", "5A", "\x1b[200~\r\x1b[18~\x03\x1b[201~"]) {
        input.emit("data", chunk(part)); await tick()
        assert.equal(view.isActive(), true)
      }
      input.emit("data", chunk("\r")); await tick()
      assert.equal(view.isActive(), false)
    })
  })
}

test("MP-08 / MP-10 standalone Escape is bounded and cancellation clears its timer", async () => {
  await withTextView(async (view, input, calls) => {
    view.present("selected"); await tick()
    input.emit("data", "\x1b"); await tick()
    assert.equal(view.isActive(), true)
    await delay(320)
    assert.equal(view.isActive(), false)
    assert.equal(calls.length, 1)
    view.present("cancelled"); await tick()
    input.emit("data", Buffer.from("\x1b")); view.cancel(); await tick()
    view.present("next"); await tick(); await delay(320)
    assert.equal(view.isActive(), true)
    assert.equal(calls.length, 2)
    input.emit("end"); await tick()
    assert.equal(view.isActive(), false)
    assert.equal(input.listenerCount("end"), 0)
  })
})

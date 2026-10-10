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

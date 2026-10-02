import assert from "node:assert/strict"
import test from "node:test"
import { PassThrough } from "node:stream"
import { readHiddenInput } from "./hidden-input.js"

function terminal(raw = false) {
  const input = Object.assign(new PassThrough(), {
    isTTY: true, isRaw: raw,
    setRawMode(enabled: boolean) { this.isRaw = enabled; return this },
  })
  const output = Object.assign(new PassThrough(), { isTTY: true })
  let rendered = ""
  output.on("data", (data) => { rendered += data.toString() })
  return { input, output, rendered: () => rendered }
}

test("hidden input excludes competing data/keypress listeners and restores them without history", async () => {
  const tty = terminal()
  const history: string[] = []
  const onData = (data: Buffer) => history.push(data.toString())
  const onKey = () => history.push("keypress")
  tty.input.on("data", onData)
  tty.input.on("keypress", onKey)
  const masks: string[] = []
  const promise = readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream,
    prompt: "Secret: ", renderMask: (mask) => masks.push(mask) })
  tty.input.emit("data", Buffer.from("test-secr"))
  tty.input.emit("keypress", "x")
  tty.input.emit("data", Buffer.from("et\r"))
  assert.equal(await promise, "test-secret")
  assert.deepEqual(history, [])
  assert.equal(tty.rendered(), "Secret: \n")
  assert.deepEqual(masks, ["", "•".repeat(9), ""])
  assert.equal(tty.input.isRaw, false)
  assert.deepEqual(tty.input.listeners("data"), [onData])
  assert.deepEqual(tty.input.listeners("keypress"), [onKey])
})

test("split UTF-8 and bracketed paste never submit embedded newlines or keep escape sequences", async () => {
  const tty = terminal(true)
  tty.input.pause()
  const promise = readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream })
  for (const byte of Buffer.from("\x1b[200~päss\nword\x1b[201~\x1b[D\x7f!\r")) {
    tty.input.emit("data", Buffer.from([byte]))
  }
  assert.equal(await promise, "päss\nwor!")
  assert.equal(tty.input.isRaw, true)
  assert.equal(tty.input.isPaused(), true)
  assert.equal(tty.rendered(), "")
})

test("shell paste framing is enabled and restored on cancellation", async () => {
  const tty = terminal()
  const promise = readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream,
    prompt: "Secret: ", manageBracketedPaste: true })
  tty.input.emit("data", Buffer.from("discard-me\x03"))
  await assert.rejects(promise, /cancelled/)
  assert.equal(tty.rendered(), "\x1b[?2004hSecret: \x1b[?2004l\n")
})

for (const exit of ["ctrl-c", "ctrl-d", "end", "close", "input-error", "output-error", "abort", "render-error", "raw-error"]) {
  test(`hidden input restores raw mode and listeners after ${exit}`, async () => {
    const tty = terminal()
    const listener = () => {}
    tty.input.on("data", listener)
    tty.input.pause()
    const abort = new AbortController()
    if (exit === "raw-error") {
      const setRaw = tty.input.setRawMode.bind(tty.input)
      tty.input.setRawMode = (enabled) => { setRaw(enabled); if (enabled) throw new Error("synthetic failure"); return tty.input }
    }
    const promise = readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream,
      signal: abort.signal, renderMask: () => { if (exit === "render-error") throw new Error("synthetic failure") } })
    if (exit === "ctrl-c") tty.input.emit("data", Buffer.from("discard-me\x03"))
    if (exit === "ctrl-d") tty.input.emit("data", Buffer.from("discard-me\x04"))
    if (exit === "end" || exit === "close") tty.input.emit(exit)
    if (exit === "input-error") tty.input.emit("error", new Error("discard-me"))
    if (exit === "output-error") tty.output.emit("error", new Error("discard-me"))
    if (exit === "abort") abort.abort()
    await assert.rejects(promise, /secret input|hidden input/)
    assert.equal(tty.input.isRaw, false)
    assert.equal(tty.input.isPaused(), true)
    assert.deepEqual(tty.input.listeners("data"), [listener])
    assert.equal(tty.input.listenerCount("error"), 0)
    assert.equal(tty.output.listenerCount("error"), 0)
    assert.equal(tty.rendered(), "")
  })
}

test("noninteractive and concurrent readers fail closed", async () => {
  const tty = terminal()
  tty.output.isTTY = false
  await assert.rejects(readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream }), /interactive TTY/)
  tty.output.isTTY = true
  const abort = new AbortController()
  const first = readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream, signal: abort.signal })
  await assert.rejects(readHiddenInput({ ...tty, input: tty.input as unknown as NodeJS.ReadStream }), /already open/)
  abort.abort()
  await assert.rejects(first, /cancelled/)
})

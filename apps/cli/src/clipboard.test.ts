import assert from "node:assert/strict"
import test from "node:test"
import { clipboardCopyMessage, copyTextToClipboard } from "./clipboard.js"

test("MP-08/MP-11 SSH never copies to the server clipboard or promises OSC 52 delivery", async () => {
  let nativeCalls = 0
  let requested = ""
  const result = await copyTextToClipboard("selected λ", { copyToClipboardOSC52: text => { requested = text; return true } }, {
    remote: true, nativeCopy: async () => { nativeCalls++ },
  })
  assert.equal(result, "requested")
  assert.equal(nativeCalls, 0)
  assert.equal(requested, "selected λ")
  assert.match(clipboardCopyMessage(result), /F7.*mouse.*drag-select.*Cmd-C/)
  assert.doesNotMatch(clipboardCopyMessage(result), /copied/)
})

test("MP-08/MP-11 local system clipboard success precedes terminal transport", async () => {
  let written = ""
  const result = await copyTextToClipboard("local text", { copyToClipboardOSC52: () => { throw Error("should not send") } }, {
    remote: false, nativeCopy: async text => { written = text },
  })
  assert.equal(result, "copied")
  assert.equal(written, "local text")
})

test("MP-08/MP-11 OpenTUI declining OSC 52 is unavailable, never a write around the renderer", async () => {
  // A real TTY that OpenTUI reports without OSC 52 support (e.g. Terminal.app).
  const tty = Object.getOwnPropertyDescriptor(process.stdout, "isTTY")
  Object.defineProperty(process.stdout, "isTTY", { value: true, configurable: true })
  try {
    for (const remote of [true, false]) {
      const unavailable = await copyTextToClipboard("payload", { copyToClipboardOSC52: () => false }, {
        remote, nativeCopy: async () => { throw Error("headless") },
      })
      assert.equal(unavailable, "unavailable")
      assert.match(clipboardCopyMessage(unavailable), /F7.*mouse.*drag-select.*Cmd-C/)
    }
  } finally {
    if (tty) Object.defineProperty(process.stdout, "isTTY", tty)
    else delete (process.stdout as { isTTY?: boolean }).isTTY
  }
})

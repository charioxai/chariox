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
  assert.match(clipboardCopyMessage(result), /unconfirmed.*native selection/)
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

test("MP-08/MP-11 terminal fallback uses the full payload; unsupported terminals get manual guidance", async () => {
  const options = { remote: false, nativeCopy: async () => { throw Error("headless") } }
  let written = ""
  assert.equal(await copyTextToClipboard("payload", { copyToClipboardOSC52: () => false }, {
    ...options, terminalCopy: text => { written = text; return true },
  }), "requested")
  assert.equal(written, "payload")
  const unavailable = await copyTextToClipboard("payload", { copyToClipboardOSC52: () => false }, {
    ...options, terminalCopy: () => false,
  })
  assert.equal(unavailable, "unavailable")
  assert.match(clipboardCopyMessage(unavailable), /native selection.*Copy/)
})

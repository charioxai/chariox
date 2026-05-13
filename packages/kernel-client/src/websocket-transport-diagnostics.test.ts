import assert from "node:assert/strict"
import test from "node:test"

import {
  extractTransportErrorMessage,
  formatTransportError,
  isWebSocketEndpoint,
} from "./websocket-transport-diagnostics.js"

test("isWebSocketEndpoint accepts websocket URLs only", () => {
  assert.equal(isWebSocketEndpoint("ws://127.0.0.1:3000"), true)
  assert.equal(isWebSocketEndpoint("wss://relay.example/ws"), true)
  assert.equal(isWebSocketEndpoint("/tmp/kernel.sock"), false)
  assert.equal(isWebSocketEndpoint("http://127.0.0.1:3000"), false)
})

test("extractTransportErrorMessage prefers nested useful fields", () => {
  assert.equal(extractTransportErrorMessage(new Error("boom")), "boom")
  assert.equal(extractTransportErrorMessage({ error: { reason: "closed" } }), "closed")
  assert.equal(extractTransportErrorMessage({ type: "unexpected-response" }), "websocket unexpected-response")
  assert.equal(extractTransportErrorMessage({ type: "error" }), null)
})

test("formatTransportError falls back to endpoint diagnostics", () => {
  assert.equal(formatTransportError({}, "ws://127.0.0.1:3000"), "websocket error at ws://127.0.0.1:3000")
})

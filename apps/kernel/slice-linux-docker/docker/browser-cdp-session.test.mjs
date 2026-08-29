import assert from "node:assert/strict"
import test from "node:test"

import { withCdpSocket } from "./browser-cdp-session.mjs"

class RespondingSocket extends EventTarget {
  readyState = 0

  constructor() {
    super()
    queueMicrotask(() => {
      this.readyState = 1
      this.dispatchEvent(new Event("open"))
    })
  }

  send(payload) {
    const request = JSON.parse(payload)
    queueMicrotask(() => this.dispatchEvent(new MessageEvent("message", {
      data: JSON.stringify({ id: request.id, result: { method: request.method } }),
    })))
  }

  close() {
    if (this.readyState === 3) return
    this.readyState = 3
    this.dispatchEvent(new Event("close"))
  }
}

class ClosingSocket extends RespondingSocket {
  send() {
    queueMicrotask(() => this.close())
  }
}

class NeverOpeningSocket extends EventTarget {
  readyState = 0

  send() {}

  close() {
    this.readyState = 3
    this.dispatchEvent(new Event("close"))
  }
}

test("CDP sessions return command responses without an unconditional settle delay", async () => {
  const startedAt = Date.now()
  const result = await withCdpSocket({
    url: "ws://example.test",
    WebSocketImpl: RespondingSocket,
    callback: (send) => send("Runtime.evaluate"),
  })

  assert.deepEqual(result, { method: "Runtime.evaluate" })
  assert.ok(Date.now() - startedAt < 200)
})

test("CDP sessions reject pending commands when Chromium closes the socket", async () => {
  await assert.rejects(withCdpSocket({
    url: "ws://example.test",
    WebSocketImpl: ClosingSocket,
    callback: (send) => send("Runtime.evaluate"),
  }), /CDP socket closed/)
})

test("CDP sessions bound connection setup", async () => {
  await assert.rejects(withCdpSocket({
    url: "ws://example.test",
    WebSocketImpl: NeverOpeningSocket,
    connectTimeoutMs: 10,
    callback: () => undefined,
  }), /CDP socket did not open in time/)
})

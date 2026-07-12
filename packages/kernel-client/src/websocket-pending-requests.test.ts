import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcError } from "./local-ipc-error.js"
import { KernelPendingRequestRegistry } from "./websocket-pending-requests.js"

test("KernelPendingRequestRegistry resolves taken requests and clears relay keys", async () => {
  const registry = new KernelPendingRequestRegistry(1_000)
  const request = registry.register<string>("request-1", "control")
  const relayKey = Buffer.from("relay-key")

  request.setRelayPrivateKey(relayKey)
  const pending = registry.take("request-1")
  assert.equal(pending?.relayPrivateKey, relayKey)
  pending?.resolve("ok")

  assert.equal(await request.promise, "ok")
  assert.equal(registry.take("request-1"), null)
})

test("KernelPendingRequestRegistry rejects requests by lane", async () => {
  const registry = new KernelPendingRequestRegistry(1_000)
  const control = registry.register<string>("control-request", "control")
  const event = registry.register<string>("event-request", "event")

  registry.rejectMatching("closed", "event")
  await assert.rejects(event.promise, /closed/)

  const pendingControl = registry.take("control-request")
  pendingControl?.resolve("still-open")
  assert.equal(await control.promise, "still-open")
})

test("KernelPendingRequestRegistry rejects write failures once", async () => {
  const registry = new KernelPendingRequestRegistry(1_000)
  const request = registry.register<string>("request-1", "control")
  const error = new LocalIpcError("write kernel request", "write failed", "write_failed", true)

  request.reject(error)
  request.reject(error)

  await assert.rejects(request.promise, /write failed/)
})

test("KernelPendingRequestRegistry supports a bounded attempt timeout", async () => {
  const registry = new KernelPendingRequestRegistry(10_000)
  const startedAt = Date.now()
  const request = registry.register<string>("request-stalled", "control", 25)

  await assert.rejects(request.promise, (error: unknown) => {
    assert.ok(error instanceof LocalIpcError)
    assert.equal(error.code, "request_timeout")
    assert.equal(error.retryable, true)
    return true
  })
  assert.ok(Date.now() - startedAt < 1_000)
})

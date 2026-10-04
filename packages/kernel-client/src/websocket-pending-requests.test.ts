import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcError } from "./local-ipc-error.js"
import { KernelPendingRequestRegistry } from "./websocket-pending-requests.js"

test("KernelPendingRequestRegistry resolves taken requests and keeps the response decryptor", async () => {
  const registry = new KernelPendingRequestRegistry(1_000)
  const request = registry.register<string>("request-1", "control")
  const decryptResponse = () => "decoded"

  request.setRelayDecryptResponse(decryptResponse)
  const pending = registry.take("request-1")
  assert.equal(pending?.relayDecryptResponse, decryptResponse)
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

test("KernelPendingRequestRegistry leaves authorization waits untimed but rejects on disconnect", async () => {
  const registry = new KernelPendingRequestRegistry(1)
  const request = registry.register<string>("authorization", "control", 0)
  await new Promise(resolve => setTimeout(resolve, 25))
  const pending = registry.take("authorization")
  assert.equal(pending?.timeout, null)
  pending?.resolve("approved")
  assert.equal(await request.promise, "approved")

  const disconnected = registry.register<string>("authorization-2", "control", 0)
  registry.rejectMatching("closed", "control")
  await assert.rejects(disconnected.promise, /closed/)
})

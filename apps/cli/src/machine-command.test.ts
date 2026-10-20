import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure } from "./agent-command-table.js"
import { failureOf, lostConnectionFailure, parseRequestBody, sendRequest } from "./machine-command.js"
import { replyWith, startFakeKernel } from "./fake-unix-kernel.test-support.js"

const refusal = (code: string) => ({ error: { code, message: `refused ${code}`, retryable: false } })

test("kernel errors keep their code and lose the transport wrapper; local connection loss is machine_connection_lost", () => {
  const refused = failureOf(new LocalIpcError("handle kernel response", "Access ended.", "kernel_access_ended"))
  assert.deepEqual([refused.code, refused.message], ["kernel_access_ended", "Access ended."])
  for (const code of ["connection_closed", "outcome_unknown", "write_failed"]) {
    assert.equal(failureOf(new LocalIpcError("handle kernel response", "gone", code)).code, "machine_connection_lost")
  }
  assert.equal(failureOf(new Error("Kernel socket not found")).code, "cli_error")
  const unreachable = failureOf(new LocalIpcError("connect kernel websocket", "connect ENOENT /k.sock", "connection_closed", true))
  assert.equal(unreachable.code, "kernel_unreachable")
  assert.match(unreachable.message, /start the kernel or pass --socket/)
  const usage = new CommandFailure("usage", "Usage: chariox request")
  assert.equal(failureOf(usage), usage)
})

test("a lost connection is definitive only when a fresh probe says the grant ended", () => {
  const ended = lostConnectionFailure(new LocalIpcError("handle kernel response", "Access ended.", "kernel_access_ended"))
  assert.equal(ended.code, "kernel_access_ended")
  assert.match(ended.message, /the access grant ended while this request was in flight; the request may or may not have run/)
  for (const probe of [null, new LocalIpcError("handle kernel response", "no access", "kernel_access_not_covered"), new Error("ECONNREFUSED")]) {
    assert.equal(lostConnectionFailure(probe).code, "machine_connection_lost")
  }
})

test("request bodies must be one JSON object and not a subscription", () => {
  assert.deepEqual(parseRequestBody('{"ListSessions":{}}'), { ListSessions: {} })
  for (const text of ["", "[]", "null", "7", "{", '{"__kernel_transport":{"type":"unsubscribe"}}']) {
    assert.throws(() => parseRequestBody(text), (error: unknown) => error instanceof CommandFailure && error.code === "usage", text)
  }
})

test("a request carries machine and command id and prints the kernel's answer", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, { response: { Pong: {} } }))
  try {
    const response = await sendRequest({ Ping: {} }, { socket: kernel.path, machine: "b3", commandId: "cmd-1", timeoutSeconds: 5 })
    assert.deepEqual(response, { Pong: {} })
    assert.equal(kernel.frames[0]!.machine, "b3")
    assert.equal(kernel.frames[0]!.command_id, "cmd-1")
    await sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 5 })
    assert.equal(kernel.frames[1]!.machine, undefined)
  } finally { await kernel.close() }
})

test("machine errors from the kernel arrive with their code", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, refusal("machine_offline")))
  try {
    await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, machine: "b3", timeoutSeconds: 5 }), (error: unknown) => failureOf(error).code === "machine_offline")
  } finally { await kernel.close() }
})

test("an unreachable kernel is not an in-flight loss and sends no probe", async () => {
  await assert.rejects(sendRequest({ Ping: {} }, { socket: "/nonexistent/k.sock", timeoutSeconds: 5 }), (error: unknown) => failureOf(error).code === "kernel_unreachable")
})

test("no answer within the timeout is machine_request_timeout", async () => {
  const kernel = await startFakeKernel(() => {})
  try {
    await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 0.2 }), (error: unknown) => failureOf(error).code === "machine_request_timeout")
  } finally { await kernel.close() }
})

test("a connection lost in flight is settled by a probe on a fresh connection", async () => {
  for (const [answer, code] of [[refusal("kernel_access_ended"), "kernel_access_ended"], [{ response: { GatewayMachinesListed: { machines: [] } } }, "machine_connection_lost"]] as const) {
    const kernel = await startFakeKernel((socket, frame) => {
      if ("Ping" in frame.request) socket.terminate()
      else replyWith(socket, frame, answer)
    })
    try {
      await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 5 }), (error: unknown) => {
        assert.equal(failureOf(error).code, code)
        return true
      })
      assert.deepEqual(kernel.frames.map(frame => Object.keys(frame.request)[0]), ["Ping", "ListGatewayMachines"])
    } finally { await kernel.close() }
  }
})

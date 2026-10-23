import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { CommandFailure, exitCodeFor } from "./agent-command-table.js"
import { failureOf, lostConnectionFailure, parseRequestBody, runKernelsCommand, sendRequest, watchKernel } from "./kernels-command.js"
import type { WebSocket } from "ws"
import { replyWith, startFakeKernel, type FakeKernelFrame } from "./fake-unix-kernel.test-support.js"

const relayStatus = (capabilities?: string[]) => ({ response: { RelayStatus: { status: { daemon_id: "d", machine_id: "m", ...(capabilities ? { capabilities } : {}) } } } })
const requestNames = (frames: { request: Record<string, unknown> }[]) => frames.map(frame => Object.keys(frame.request)[0])

const refusal = (code: string) => ({ error: { code, message: `refused ${code}`, retryable: false } })

test("kernel errors keep their code and lose the transport wrapper; local connection loss is kernel_connection_lost", () => {
  const refused = failureOf(new LocalIpcError("handle kernel response", "Access ended.", "kernel_access_ended"))
  assert.deepEqual([refused.code, refused.message], ["kernel_access_ended", "Access ended."])
  for (const code of ["connection_closed", "outcome_unknown", "write_failed"]) {
    assert.equal(failureOf(new LocalIpcError("handle kernel response", "gone", code)).code, "kernel_connection_lost")
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
    assert.equal(lostConnectionFailure(probe).code, "kernel_connection_lost")
  }
})

test("request bodies must be one JSON object and not a subscription", () => {
  assert.deepEqual(parseRequestBody('{"ListSessions":{}}'), { ListSessions: {} })
  for (const text of ["", "[]", "null", "7", "{", '{"__kernel_transport":{"type":"unsubscribe"}}']) {
    assert.throws(() => parseRequestBody(text), (error: unknown) => error instanceof CommandFailure && error.code === "usage", text)
  }
})

test("a kernel request is never written when the local kernel cannot forward", async () => {
  for (const answer of [relayStatus(), relayStatus(["disposable_worker_control_v1"]), refusal("invalid_request")]) {
    const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, answer))
    try {
      await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, kernel: "b3", timeoutSeconds: 5 }), (error: unknown) => {
        const failure = failureOf(error)
        assert.equal(failure.code, "kernel_not_available")
        assert.equal(failure.message, "This computer's Chariox kernel cannot forward to other kernels; update it.")
        return true
      })
      assert.deepEqual(requestNames(kernel.frames), ["RelayStatus"])
    } finally { await kernel.close() }
  }
})

test("a grant refusal on the preflight is the answer, not an outdated kernel", async () => {
  for (const code of ["kernel_access_ended", "kernel_access_not_covered"]) {
    const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, refusal(code)))
    try {
      await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, kernel: "b3", timeoutSeconds: 5 }), (error: unknown) => failureOf(error).code === code)
      assert.deepEqual(requestNames(kernel.frames), ["RelayStatus"])
    } finally { await kernel.close() }
  }
})

test("a kernel request preflights the gateway capability on a client without kernel; local requests skip it", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, "RelayStatus" in frame.request ? relayStatus(["kernel_gateway_v1"]) : { response: { Pong: {} } }))
  try {
    await sendRequest({ Ping: {} }, { socket: kernel.path, kernel: "b3", timeoutSeconds: 5 })
    assert.deepEqual(requestNames(kernel.frames), ["RelayStatus", "Ping"])
    assert.deepEqual(kernel.frames.map(frame => frame.kernel), [undefined, "b3"])
    await sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 5 })
    assert.deepEqual(requestNames(kernel.frames), ["RelayStatus", "Ping", "Ping"])
  } finally { await kernel.close() }
})

test("a request carries kernel and command id and prints the kernel's answer", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, "RelayStatus" in frame.request ? relayStatus(["kernel_gateway_v1"]) : { response: { Pong: {} } }))
  try {
    const response = await sendRequest({ Ping: {} }, { socket: kernel.path, kernel: "b3", commandId: "cmd-1", timeoutSeconds: 5 })
    assert.deepEqual(response, { Pong: {} })
    assert.equal(kernel.frames[1]!.kernel, "b3")
    assert.equal(kernel.frames[1]!.command_id, "cmd-1")
    await sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 5 })
    assert.equal(kernel.frames[2]!.kernel, undefined)
  } finally { await kernel.close() }
})

test("AttachToSession requires an explicit command id before any connection", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, "RelayStatus" in frame.request ? relayStatus(["kernel_gateway_v1"]) : { response: { SessionAttached: { attachment: { id: "unexpected" } } } }))
  try {
    await assert.rejects(sendRequest({ AttachToSession: { session_id: "room", client_id: "lane", capability_level: "FullTerminal" } }, { socket: kernel.path, kernel: "b3", timeoutSeconds: 5 }), (error: unknown) => error instanceof CommandFailure && error.code === "command_id_required" && error.message.includes("--command-id"))
    assert.equal(kernel.frames.length, 0)
  } finally { await kernel.close() }
})

test("kernel errors from the kernel arrive with their code", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, "RelayStatus" in frame.request ? relayStatus(["kernel_gateway_v1"]) : refusal("kernel_offline")))
  try {
    await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, kernel: "b3", timeoutSeconds: 5 }), (error: unknown) => failureOf(error).code === "kernel_offline")
  } finally { await kernel.close() }
})

test("an unreachable kernel is not an in-flight loss and sends no probe", async () => {
  await assert.rejects(sendRequest({ Ping: {} }, { socket: "/nonexistent/k.sock", timeoutSeconds: 5 }), (error: unknown) => failureOf(error).code === "kernel_unreachable")
})

test("no answer within the timeout is kernel_request_timeout", async () => {
  const kernel = await startFakeKernel(() => {})
  try {
    await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 0.2 }), (error: unknown) => {
      const failure = failureOf(error)
      assert.equal(failure.code, "kernel_request_timeout")
      assert.match(failure.message, /retry once with the same --command-id/, "the message names the next step")
      return true
    })
  } finally { await kernel.close() }
})

test("a connection lost in flight is settled by a probe on a fresh connection", async () => {
  for (const [answer, code] of [[refusal("kernel_access_ended"), "kernel_access_ended"], [{ response: { GatewayKernelsListed: { kernels: [] } } }, "kernel_connection_lost"]] as const) {
    const kernel = await startFakeKernel((socket, frame) => {
      if ("Ping" in frame.request) socket.terminate()
      else replyWith(socket, frame, answer)
    })
    try {
      await assert.rejects(sendRequest({ Ping: {} }, { socket: kernel.path, timeoutSeconds: 5 }), (error: unknown) => {
        assert.equal(failureOf(error).code, code)
        return true
      })
      assert.deepEqual(kernel.frames.map(frame => Object.keys(frame.request)[0]), ["Ping", "ListGatewayKernels"])
    } finally { await kernel.close() }
  }
})

const watchOptions = { kernel: "b3", sessionId: "s-1", attachmentId: "att-1", from: 4, socket: "" }
const subscribeAck = (frame: { request_id: string }) => ({ type: "response", request_id: frame.request_id, kernel: "b3", response: { ok: true }, error: null })
const eventFrame = (eventId: number) => ({ type: "event", event_id: eventId, kernel: "b3", event: { event: "client_catalog_changed", revision: eventId } })

async function startWatchKernel(onSubscribe: (socket: WebSocket, frame: FakeKernelFrame) => void) {
  return startFakeKernel((socket, frame) => {
    if (frame.type === "subscribe") onSubscribe(socket, frame)
    else replyWith(socket, frame, relayStatus(["kernel_gateway_v1"]))
  })
}

test("a watch prints every event frame and ends with the kernel's kernel_connection_lost and its resume hint", async () => {
  const kernel = await startWatchKernel((socket, frame) => {
    socket.send(JSON.stringify(subscribeAck(frame)))
    socket.send(JSON.stringify(eventFrame(5)))
    socket.send(JSON.stringify(eventFrame(6)))
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, kernel: "b3", response: null, error: { code: "kernel_connection_lost", message: "Connection to b3 lost. Subscribe again with resume_from_event_id 6.", retryable: true } }))
  })
  const lines: string[] = []
  try {
    await assert.rejects(watchKernel({ ...watchOptions, socket: kernel.path }, line => lines.push(line)), (error: unknown) => {
      const failure = failureOf(error)
      assert.equal(failure.code, "kernel_connection_lost")
      assert.match(failure.message, /^Connection to b3 lost\. Subscribe again with resume_from_event_id 6\.$/)
      assert.equal(exitCodeFor(failure.code), 5)
      return true
    })
    assert.deepEqual(lines.map(line => JSON.parse(line)), [eventFrame(5), eventFrame(6)])
    assert.deepEqual(requestNames(kernel.frames.filter(frame => frame.type === "request")), ["RelayStatus"])
    const subscribe = kernel.frames.find(frame => frame.type === "subscribe")!
    assert.deepEqual([subscribe.kernel, subscribe.resume_from_event_id], ["b3", 4])
  } finally { await kernel.close() }
})

test("a watch fails with the kernel error of a refused subscription", async () => {
  const kernel = await startWatchKernel((socket, frame) => replyWith(socket, frame, refusal("kernel_offline")))
  try {
    await assert.rejects(watchKernel({ ...watchOptions, socket: kernel.path }, () => {}), (error: unknown) => failureOf(error).code === "kernel_offline")
  } finally { await kernel.close() }
})

test("a watch is never subscribed when the local kernel cannot forward", async () => {
  const kernel = await startFakeKernel((socket, frame) => replyWith(socket, frame, relayStatus()))
  try {
    await assert.rejects(watchKernel({ ...watchOptions, socket: kernel.path }, () => {}), (error: unknown) => failureOf(error).code === "kernel_not_available")
    assert.deepEqual(kernel.frames.map(frame => frame.type), ["request"])
  } finally { await kernel.close() }
})

test("a watch whose grant was revoked says the access ended, not that the connection was lost", async () => {
  const kernel = await startFakeKernel((socket, frame) => {
    if (frame.type === "subscribe") {
      socket.send(JSON.stringify(subscribeAck(frame)))
      setTimeout(() => socket.terminate(), 50)
    } else if ("RelayStatus" in frame.request) replyWith(socket, frame, relayStatus(["kernel_gateway_v1"]))
    else replyWith(socket, frame, refusal("kernel_access_ended"))
  })
  try {
    await assert.rejects(watchKernel({ ...watchOptions, socket: kernel.path }, () => {}), (error: unknown) => failureOf(error).code === "kernel_access_ended")
  } finally { await kernel.close() }
})

test("a watch whose local kernel connection drops names the last event to resume from", async () => {
  const kernel = await startWatchKernel((socket, frame) => {
    socket.send(JSON.stringify(subscribeAck(frame)))
    socket.send(JSON.stringify(eventFrame(9)))
    setTimeout(() => socket.terminate(), 50)
  })
  try {
    await assert.rejects(watchKernel({ ...watchOptions, socket: kernel.path }, () => {}), (error: unknown) => {
      const failure = failureOf(error)
      assert.equal(failure.code, "kernel_connection_lost")
      assert.match(failure.message, /--from 9/)
      return true
    })
  } finally { await kernel.close() }
})

test("terminal send --wait-ms keeps its request open for the whole wait, whatever the default deadline", async () => {
  const summary = { terminal_id: "t1", session_id: "s1", title: "ops", output_offset: 5 }
  const roomAnswer = (result: unknown) => ({ response: { RoomTerminal: { result } } })
  const kernel = await startFakeKernel((socket, frame) => {
    const operation = (frame.request as any).RoomTerminal?.operation?.operation
    if ("RelayStatus" in frame.request) replyWith(socket, frame, relayStatus(["kernel_gateway_v1"]))
    else if (operation === "snapshot") replyWith(socket, frame, roomAnswer({ result: "snapshot", snapshot: { terminal: summary, output_offset: 5, screen_text: "" } }))
    else if (operation === "input") replyWith(socket, frame, roomAnswer({ result: "input", input: { accepted: true, driver: {} } }))
    else setTimeout(() => replyWith(socket, frame, roomAnswer({ result: "output", output: { terminal: summary, data_base64: Buffer.from("done\r\n").toString("base64"), next_offset: 11, gap: false } })), 1500)
  })
  const printed: string[] = []
  const log = console.log
  console.log = (line: string) => { printed.push(line) }
  try {
    await runKernelsCommand(["terminal", "send", "ls", "--kernel", "b3", "--session", "s1", "--terminal", "t1", "--wait-ms", "3000", "--timeout", "1", "--socket", kernel.path])
  } finally { console.log = log; await kernel.close() }
  assert.equal(JSON.parse(printed.at(-1)!).text, "done\r\n")
})

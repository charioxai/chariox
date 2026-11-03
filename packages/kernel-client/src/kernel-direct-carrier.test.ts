import assert from "node:assert/strict"
import test from "node:test"
import { KernelDirectCarrier } from "./kernel-direct-carrier.js"

class Socket extends EventTarget {
  readyState = 1
  bufferedAmount = 0
  readonly sent: unknown[] = []
  send(data: string): void { this.sent.push(JSON.parse(data)) }
  receive(data: unknown): void { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(data) })) }
  receiveBytes(data: ArrayBuffer): void { this.dispatchEvent(new MessageEvent("message", { data })) }
  close(): void { this.readyState = 3; this.dispatchEvent(Object.assign(new Event("close"), { code: 1006, reason: "lost" })) }
}

test("MP-08/MP-11 compact frames retain routing and the authenticated kernel key", () => {
  const socket = new Socket()
  const carrier = new KernelDirectCarrier(socket as unknown as WebSocket, "pinned-key", () => {})
  const lane = carrier.openLane(), received: unknown[] = []
  lane.addEventListener("message", event => received.push(JSON.parse((event as MessageEvent).data)))
  lane.send(JSON.stringify({ kind: "client_request", request_id: "r" }))
  const frame = new Uint8Array(36)
  frame.set([67, 88, 76, 50, 0, 0, 1]); frame[19] = 114
  socket.receiveBytes(frame.buffer)
  assert.deepEqual(received, [{ kind: "client_response", request_id: "r", error: null,
    encrypted_response: { sender_public_key: "pinned-key", nonce: "AAAAAAAAAAAAAAAA", ciphertext: "AAAAAAAAAAAAAAAAAAAAAA==" } }])
  lane.close()
})

test("MP-11 compact frames cannot replace the pinned key or smuggle routing fields", () => {
  for (const header of [{ r: "r", n: "AAAAAAAAAAAAAAAA", sender_public_key: "other" }, { r: "r", s: "s", n: "AAAAAAAAAAAAAAAA" }, { s: "s", e: -1, n: "AAAAAAAAAAAAAAAA" }, { r: "r", n: "invalid" }]) {
    const socket = new Socket(), carrier = new KernelDirectCarrier(socket as unknown as WebSocket, "pinned", () => {})
    const lane = carrier.openLane()
    const encoded = new TextEncoder().encode(JSON.stringify(header)), frame = new Uint8Array(8 + encoded.length + 16)
    frame.set([67, 88, 76, 49]); new DataView(frame.buffer).setUint32(4, encoded.length); frame.set(encoded, 8)
    socket.receiveBytes(frame.buffer)
    assert.equal(lane.readyState, 3)
  }
})

test("MP-08/MP-11 shared carrier isolates replies, subscriptions and lane retirement", () => {
  const socket = new Socket()
  const carrier = new KernelDirectCarrier(socket as unknown as WebSocket, "key", () => assert.fail("intentional retirement is not loss"))
  const a = carrier.openLane(), b = carrier.openLane()
  const received: unknown[][] = [[], []]
  for (const [index, lane] of [a, b].entries()) lane.addEventListener("message", event => received[index]!.push(JSON.parse((event as MessageEvent).data)))
  a.send(JSON.stringify({ kind: "client_request", request_id: "a" }))
  b.send(JSON.stringify({ kind: "client_request", request_id: "b" }))
  a.send(JSON.stringify({ kind: "client_subscribe", request_id: "subscribe-a", subscription_id: "sa", client_public_key: "key-a" }))
  b.send(JSON.stringify({ kind: "client_subscribe", subscription_id: "sb", client_public_key: "key-b" }))
  socket.receive({ kind: "client_response", request_id: "subscribe-a" })
  socket.receive({ kind: "client_response", request_id: "b" })
  socket.receive({ kind: "client_event", subscription_id: "sa" })
  assert.deepEqual(received, [[{ kind: "client_response", request_id: "subscribe-a" }, { kind: "client_event", subscription_id: "sa" }], [{ kind: "client_response", request_id: "b" }]])
  a.close()
  assert.equal(socket.readyState, 1)
  assert.equal((socket.sent.at(-1) as { subscription_id: string }).subscription_id, "sa")
  assert.equal((socket.sent.at(-1) as { client_public_key: string }).client_public_key, "key-a")
  socket.receive({ kind: "client_response", request_id: "a" })
  socket.receive({ kind: "client_event", subscription_id: "sa" })
  socket.receive({ kind: "client_event", subscription_id: "sb" })
  assert.equal(received[0]!.length, 2)
  assert.equal(received[1]!.length, 2)
  b.close()
  assert.equal(socket.readyState, 3)
})

test("MP-11 a lane cannot take another lane's routing id or unsubscribe its stream", () => {
  const socket = new Socket()
  const carrier = new KernelDirectCarrier(socket as unknown as WebSocket, "key", () => {})
  const a = carrier.openLane(), b = carrier.openLane()
  a.send(JSON.stringify({ kind: "client_request", request_id: "a" }))
  a.send(JSON.stringify({ kind: "client_subscribe", subscription_id: "sa", client_public_key: "key-a" }))
  assert.throws(() => b.send(JSON.stringify({ kind: "client_request", request_id: "a" })), /collision/)
  assert.throws(() => b.send(JSON.stringify({ kind: "client_subscribe", subscription_id: "sa", client_public_key: "key-a" })), /collision/)
  assert.throws(() => b.send(JSON.stringify({ kind: "client_unsubscribe", subscription_id: "sa", client_public_key: "key-a" })), /does not own/)
  a.close(); b.close()
})

test("MP-08 physical loss retires every lane and records recovery before callbacks", () => {
  const socket = new Socket(), order: string[] = []
  const carrier = new KernelDirectCarrier(socket as unknown as WebSocket, "key", () => order.push("lost"))
  const a = carrier.openLane(), b = carrier.openLane()
  for (const lane of [a, b]) lane.addEventListener("close", () => order.push("closed"))
  socket.close()
  assert.deepEqual(order, ["lost", "closed", "closed"])
  assert.equal(a.readyState, 3); assert.equal(b.readyState, 3)
  assert.throws(() => carrier.openLane(), /closed/)
})

test("MP-08 physical errors record recovery before lane cancellation", () => {
  const socket = new Socket(), order: string[] = []
  const carrier = new KernelDirectCarrier(socket as unknown as WebSocket, "key", () => order.push("lost"))
  for (const lane of [carrier.openLane(), carrier.openLane()]) {
    lane.addEventListener("error", () => lane.close())
    lane.addEventListener("close", () => order.push("closed"))
  }
  socket.dispatchEvent(new Event("error"))
  assert.deepEqual(order, ["lost", "closed", "closed"])
})

import assert from "node:assert/strict"
import test from "node:test"

import {
  createRelayKeypair,
  decryptRelayPayload,
  encryptRelayPayload,
  type RelayKeypair,
} from "./browser-relay-crypto.js"
import { createRelayKeypair as createNativeRelayKeypair, RelayClientIdentity } from "./relay-crypto.js"
import {
  openSelkiesDisplayStream,
  type DisplayKernelClient,
  type DisplayWebSocket,
  type DisplayWebSocketConstructor,
  type SelkiesDisplayStream,
} from "./display-stream.js"

const DISPLAY_URL = "ws://display.example.test/stream"

class ControlledWebSocket implements DisplayWebSocket {
  static latest: ControlledWebSocket | undefined

  readonly sent: Uint8Array[] = []
  readyState = 0
  closeCount = 0
  private readonly listeners = new Map<string, Set<(...args: unknown[]) => void>>()

  constructor(readonly url: string) {
    ControlledWebSocket.latest = this
    queueMicrotask(() => {
      this.readyState = 1
      this.emit("open")
    })
  }

  send(data: Uint8Array | ArrayBuffer) {
    this.sent.push(data instanceof Uint8Array ? new Uint8Array(data) : new Uint8Array(data))
  }

  close() {
    if (this.readyState === 3) return
    this.closeCount += 1
    this.readyState = 3
    this.emit("close")
  }

  on(type: string, listener: (...args: unknown[]) => void) {
    const listeners = this.listeners.get(type) ?? new Set<(...args: unknown[]) => void>()
    listeners.add(listener)
    this.listeners.set(type, listeners)
  }

  off(type: string, listener: (...args: unknown[]) => void) {
    this.listeners.get(type)?.delete(listener)
  }

  removeListener(type: string, listener: (...args: unknown[]) => void) {
    this.listeners.get(type)?.delete(listener)
  }

  emit(type: string, ...args: unknown[]) {
    for (const listener of this.listeners.get(type) ?? []) listener(...args)
  }
}

const controlledWebSocket = ControlledWebSocket as unknown as DisplayWebSocketConstructor

async function openFixture() {
  const worker = await createRelayKeypair()
  const client: DisplayKernelClient = {
    async send<TResponse>() {
      return {
        SliceDisplayEndpoint: {
          endpoint: {
            slice_id: "slice-1",
            kind: "selkies",
            url: DISPLAY_URL,
            access: "tunnel",
            stream_protocol: "chariox-display-v1",
            stream_id: "stream-1",
            peer_public_key: worker.publicKeyBase64,
          },
        },
      } as TResponse
    },
  }
  const stream = await openSelkiesDisplayStream({
    client,
    sliceId: "slice-1",
    sessionId: "room-1",
    attachmentId: "attachment-1",
    webSocket: controlledWebSocket,
  })
  const socket = ControlledWebSocket.latest
  assert.ok(socket)
  return { stream, socket, worker }
}

async function encryptedFragment(
  stream: SelkiesDisplayStream,
  worker: RelayKeypair,
  fragment: Record<string, unknown>,
) {
  const encrypted = await encryptRelayPayload(
    stream.viewerPublicKey,
    JSON.stringify({
      protocol: "chariox-display-v1",
      stream_id: "stream-1",
      sender: "kernel",
      ...fragment,
    }),
    worker,
  )
  return new TextEncoder().encode(JSON.stringify(encrypted.payload))
}

function textFragment(sequence: number, text: string) {
  return {
    sequence,
    kind: "text",
    final_fragment: true,
    data_base64: Buffer.from(text, "utf8").toString("base64"),
  }
}

test("remote display authorization and stream crypto reuse the paired CLI key", async () => {
  const worker = await createRelayKeypair()
  const cliKeypair = createNativeRelayKeypair()
  const identity = new RelayClientIdentity(cliKeypair.privateKey)
  let requestedViewerKey: string | null = null
  const client: DisplayKernelClient = {
    isRelayTransport: () => true,
    getRelayClientIdentity: () => identity,
    async send<TResponse>(request: unknown) {
      const value = request as { GetSliceDisplayEndpoint?: { viewer_public_key?: string } }
      requestedViewerKey = value.GetSliceDisplayEndpoint?.viewer_public_key ?? null
      return {
        SliceDisplayEndpoint: {
          endpoint: {
            slice_id: "slice-1",
            kind: "selkies",
            url: DISPLAY_URL,
            access: "tunnel",
            stream_protocol: "chariox-display-v1",
            stream_id: "stream-1",
            peer_public_key: worker.publicKeyBase64,
          },
        },
      } as TResponse
    },
  }
  const stream = await openSelkiesDisplayStream({
    client,
    sliceId: "slice-1",
    sessionId: "room-1",
    attachmentId: "attachment-1",
    webSocket: controlledWebSocket,
  })
  const socket = ControlledWebSocket.latest
  assert.ok(socket)
  assert.equal(requestedViewerKey, identity.publicKeyBase64)
  assert.equal(stream.viewerPublicKey, identity.publicKeyBase64)

  await stream.sendControl("START_VIDEO")
  const sent = JSON.parse(new TextDecoder().decode(socket.sent[0]!))
  assert.equal(sent.sender_public_key, identity.publicKeyBase64)
  const sentFragment = JSON.parse(await decryptRelayPayload(worker.privateKey, sent, identity.publicKeyBase64))
  assert.equal(sentFragment.sender, "viewer")
  assert.equal(Buffer.from(sentFragment.data_base64, "base64").toString("utf8"), "START_VIDEO")

  socket.emit("message", await encryptedFragment(stream, worker, textFragment(0, "same key consumed")), true)
  const received = await stream.receive({ timeoutMs: 1_000 })
  assert.equal(new TextDecoder().decode(received.data), "same key consumed")
  await stream.close()
  assert.equal(socket.closeCount, 1)
  cliKeypair.privateKey.fill(0)
})

test("remote display requires a paired CLI key before allocating an endpoint", async () => {
  let sends = 0
  const client: DisplayKernelClient = {
    isRelayTransport: () => true,
    async send<TResponse>() {
      sends += 1
      return {} as TResponse
    },
  }

  await assert.rejects(openSelkiesDisplayStream({
    client,
    sliceId: "slice-1",
    sessionId: "room-1",
    attachmentId: "attachment-1",
    webSocket: controlledWebSocket,
  }), /paired key-bound relay identity/)
  assert.equal(sends, 0)
})

function binaryFragment(sequence: number, bytes = Buffer.from([4, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10])) {
  return {
    sequence,
    kind: "binary",
    final_fragment: true,
    data_base64: bytes.toString("base64"),
  }
}

async function waitForClose(socket: ControlledWebSocket, timeoutMs = 250) {
  const deadline = Date.now() + timeoutMs
  while (socket.closeCount === 0 && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
  assert.ok(socket.closeCount > 0, "stream must close after a terminal receive failure")
}

async function withDeadline<T>(operation: Promise<T>, timeoutMs = 250): Promise<T> {
  return Promise.race([
    operation,
    new Promise<T>((_, reject) => setTimeout(() => reject(new Error("test operation deadline exceeded")), timeoutMs)),
  ])
}

test("display receive timeout rejects its waiter and closes the stream", async () => {
  const { stream, socket } = await openFixture()
  await assert.rejects(
    withDeadline(stream.receive({ timeoutMs: 5 })),
    /did not receive display data before the deadline/,
  )
  assert.equal(socket.closeCount, 1)
  await stream.close()
})

test("display receive abort rejects its waiter and closes the stream", async () => {
  const { stream, socket } = await openFixture()
  const controller = new AbortController()
  const pending = stream.receive({ signal: controller.signal })
  controller.abort()
  await assert.rejects(withDeadline(pending), /was aborted display receive/)
  assert.equal(socket.closeCount, 1)
  await stream.close()
})

test("display ingress preserves packet order when the first decode is delayed", async () => {
  const { stream, socket, worker } = await openFixture()
  const first = await encryptedFragment(stream, worker, textFragment(0, "first"))
  const second = await encryptedFragment(stream, worker, textFragment(1, "second"))
  const delayedFirst = {
    async arrayBuffer() {
      await new Promise((resolve) => setTimeout(resolve, 15))
      return first.buffer.slice(first.byteOffset, first.byteOffset + first.byteLength)
    },
  }
  socket.emit("message", delayedFirst, true)
  socket.emit("message", second, true)
  const messages = await Promise.all([stream.receive({ timeoutMs: 250 }), stream.receive({ timeoutMs: 250 })])
  assert.deepEqual(messages.map((message) => new TextDecoder().decode(message.data)), ["first", "second"])
  await stream.close()
})

test("display ingress backlog is bounded before a stalled decode and discarded after close", async () => {
  const { stream, socket, worker } = await openFixture()
  const first = await encryptedFragment(stream, worker, textFragment(0, "first"))
  let releaseFirst = () => {}
  let firstDecodeStarted = false
  const stalledFirst = {
    size: first.byteLength,
    async arrayBuffer() {
      firstDecodeStarted = true
      return new Promise<ArrayBuffer>((resolve) => {
        releaseFirst = () => resolve(first.buffer.slice(first.byteOffset, first.byteOffset + first.byteLength))
      })
    },
  }
  socket.emit("message", stalledFirst, true)
  await new Promise<void>((resolve) => queueMicrotask(resolve))
  assert.equal(firstDecodeStarted, true)

  const subsequent = await Promise.all(Array.from({ length: 17 }, async (_, index) => (
    encryptedFragment(stream, worker, textFragment(index + 1, `queued-${index}`))
  )))
  try {
    for (const packet of subsequent) socket.emit("message", packet, true)
    await waitForClose(socket)
    assert.equal(socket.closeCount, 1)
    releaseFirst()
    await assert.rejects(withDeadline(stream.receive()), /ingress exceeded its bounded capacity/)
    await new Promise<void>((resolve) => queueMicrotask(resolve))
    await assert.rejects(withDeadline(stream.receive()), /ingress exceeded its bounded capacity/)
  } finally {
    releaseFirst()
    await stream.close()
  }
})

test("display ingress byte budget closes before the pending packet count", async () => {
  const { stream, socket } = await openFixture()
  let releaseFirst = () => {}
  const stalledFirst = {
    size: 1,
    async arrayBuffer() {
      return new Promise<ArrayBuffer>((resolve) => {
        releaseFirst = () => resolve(new ArrayBuffer(1))
      })
    },
  }
  socket.emit("message", stalledFirst, true)
  await new Promise<void>((resolve) => queueMicrotask(resolve))
  try {
    for (let index = 0; index < 4; index += 1) {
      socket.emit("message", new Uint8Array(1024 * 1024), true)
    }
    await waitForClose(socket)
    assert.equal(socket.closeCount, 1)
    releaseFirst()
    await assert.rejects(withDeadline(stream.receive()), /ingress exceeded its bounded capacity/)
  } finally {
    releaseFirst()
    await stream.close()
  }
})

test("display close discards a decode that completes after closure", async () => {
  const { stream, socket, worker } = await openFixture()
  const first = await encryptedFragment(stream, worker, textFragment(0, "first"))
  let releaseFirst = () => {}
  let firstDecodeStarted = false
  const stalledFirst = {
    size: first.byteLength,
    async arrayBuffer() {
      firstDecodeStarted = true
      return new Promise<ArrayBuffer>((resolve) => {
        releaseFirst = () => resolve(first.buffer.slice(first.byteOffset, first.byteOffset + first.byteLength))
      })
    },
  }
  socket.emit("message", stalledFirst, true)
  await new Promise<void>((resolve) => queueMicrotask(resolve))
  assert.equal(firstDecodeStarted, true)
  const pending = stream.receive({ timeoutMs: 250 })
  const closing = stream.close()
  await assert.rejects(withDeadline(pending), /was closed/)
  releaseFirst()
  await closing
  await new Promise<void>((resolve) => queueMicrotask(resolve))
  await assert.rejects(withDeadline(stream.receive()), /closed/)
})

test("concurrent display controls reserve distinct ordered sequences", async () => {
  const { stream, socket, worker } = await openFixture()
  await Promise.all([stream.sendControl("START_VIDEO"), stream.sendControl("REQUEST_KEYFRAME")])
  const fragments = await Promise.all(socket.sent.map(async (bytes) => {
    const payload = JSON.parse(new TextDecoder().decode(bytes))
    return JSON.parse(await decryptRelayPayload(worker.privateKey, payload, stream.viewerPublicKey))
  }))
  assert.deepEqual(fragments.map((fragment) => fragment.sequence), [0, 1])
  assert.deepEqual(
    fragments.map((fragment) => Buffer.from(fragment.data_base64, "base64").toString("utf8")),
    ["START_VIDEO", "REQUEST_KEYFRAME"],
  )
  await stream.close()
})

test("display receive buffering is bounded and closes on overflow", async () => {
  const { stream, socket, worker } = await openFixture()
  for (let sequence = 0; sequence < 17; sequence += 1) {
    socket.emit("message", await encryptedFragment(stream, worker, binaryFragment(sequence)), true)
  }
  await waitForClose(socket)
  await assert.rejects(withDeadline(stream.receive()), /receive buffer exceeded/)
  await stream.close()
})

test("display protocol mismatches and oversized packets permanently close the stream", async () => {
  const cases: Array<{ name: string; fragment: Record<string, unknown> }> = [
    { name: "wrong stream", fragment: { ...textFragment(0, "wrong"), stream_id: "other-stream" } },
    { name: "wrong sender", fragment: { ...textFragment(0, "wrong"), sender: "viewer" } },
    { name: "wrong sequence", fragment: textFragment(1, "wrong") },
    { name: "oversized packet", fragment: binaryFragment(0, Buffer.alloc(128 * 1024, 0x2a)) },
  ]

  for (const { fragment } of cases) {
    const { stream, socket, worker } = await openFixture()
    socket.emit("message", await encryptedFragment(stream, worker, fragment), true)
    await waitForClose(socket)
    await assert.rejects(withDeadline(stream.receive()), /display stream|display packet|display fragments/)
    await stream.close()
  }
})

import test from "node:test"
import assert from "node:assert/strict"
import {
  createCipheriv,
  createDecipheriv,
  createECDH,
  hkdfSync,
  randomBytes,
} from "node:crypto"
import type { AddressInfo } from "node:net"
import { once } from "node:events"

import { WebSocketServer } from "ws"

import { LocalIpcClient, type KernelEvent } from "./ipc.js"

type TestEncryptedRelayPayload = {
  sender_public_key: string
  nonce: string
  ciphertext: string
}

test("LocalIpcClient uses websocket request and subscription frames", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const receivedFrames: Array<Record<string, unknown>> = []
  server.on("connection", (socket) => {
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      receivedFrames.push(frame)

      if (frame.type === "request") {
        socket.send(JSON.stringify({
          type: "response",
          request_id: frame.request_id,
          response: { ok: true, echoed: frame.request },
          error: null,
        }))
        return
      }

      if (frame.type === "subscribe") {
        socket.send(JSON.stringify({
          type: "response",
          request_id: frame.request_id,
          response: { ok: true },
          error: null,
        }))
        socket.send(JSON.stringify({
          type: "event",
          event_id: 1,
          event: {
            event: "replay_gap",
            session_id: frame.session_id,
            requested_from_event_id: 1,
            first_retained_event_id: null,
            latest_event_id: null,
            message: "Replay cursor is outside the retained kernel event window; refresh the session projection.",
          },
        }))
        socket.send(JSON.stringify({
          type: "event",
          event_id: 2,
          event: {
            event: "session_snapshot",
            session: { id: frame.session_id, attachment_ids: [frame.attachment_id] },
            provider_run: null,
          },
        }))
        return
      }

      if (frame.type === "unsubscribe") {
        socket.send(JSON.stringify({
          type: "response",
          request_id: frame.request_id,
          response: { ok: true },
          error: null,
        }))
      }
    })
  })

  const client = new LocalIpcClient(endpoint)
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  const response = await client.send<{ ok: boolean; echoed: unknown }>({
    hello: "world",
  })
  assert.equal(response.ok, true)
  assert.deepEqual(response.echoed, { hello: "world" })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  await new Promise((resolve) => setTimeout(resolve, 25))
  await client.unsubscribeFromKernelEvents()

  assert.equal(receivedFrames[0]?.type, "request")
  assert.equal(receivedFrames[1]?.type, "subscribe")
  assert.equal(receivedFrames[2]?.type, "unsubscribe")
  assert.equal(receivedFrames[1]?.resume_from_event_id, null)
  assert.deepEqual(events, [
    {
      event: "replay_gap",
      session_id: "session-1",
      requested_from_event_id: 1,
      first_retained_event_id: null,
      latest_event_id: null,
      message: "Replay cursor is outside the retained kernel event window; refresh the session projection.",
    },
    {
      event: "session_snapshot",
      session: { id: "session-1", attachment_ids: ["attachment-1"] },
      provider_run: null,
    },
  ])
})

test("LocalIpcClient emits transport_closed when websocket closes", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  server.on("connection", (socket) => {
    socket.close()
  })

  const client = new LocalIpcClient(endpoint)
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await assert.rejects(
    client.send({ hello: "world" }),
    /kernel transport `connect kernel websocket` failed|kernel transport `kernel websocket` failed/,
  )

  await new Promise((resolve) => setTimeout(resolve, 25))
  assert.equal(events.at(-1)?.event, "transport_closed")
})

test("LocalIpcClient preserves websocket close reasons in transport_closed events", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  server.on("connection", (socket) => {
    socket.close(1008, "kernel transport overloaded; reconnecting")
  })

  const client = new LocalIpcClient(endpoint)
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await assert.rejects(
    client.send({ hello: "world" }),
    /kernel transport `connect kernel websocket` failed|kernel transport `kernel websocket` failed/,
  )

  await new Promise((resolve) => setTimeout(resolve, 25))
  assert.deepEqual(events.at(-1), {
    event: "transport_closed",
    message: "kernel transport overloaded; reconnecting",
  })
})

test("LocalIpcClient reconnects and resubscribes with the last received event id", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const subscribeFrames: Array<Record<string, unknown>> = []
  let connectionCount = 0
  server.on("connection", (socket) => {
    connectionCount += 1
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      if (frame.type === "subscribe") {
        subscribeFrames.push(frame)
        socket.send(JSON.stringify({
          type: "response",
          request_id: frame.request_id,
          response: { ok: true },
          error: null,
        }))
        socket.send(JSON.stringify({
          type: "event",
          event_id: connectionCount,
          event: {
            event: "heartbeat",
            session_id: frame.session_id,
          },
        }))
        if (connectionCount === 1) {
          setTimeout(() => socket.close(), 10)
        }
      }
    })
  })

  const client = new LocalIpcClient(endpoint)
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  await new Promise((resolve) => setTimeout(resolve, 600))

  assert.equal(subscribeFrames.length >= 2, true)
  assert.equal(subscribeFrames[0]?.resume_from_event_id, null)
  assert.equal(subscribeFrames[1]?.resume_from_event_id, 1)
  assert.equal(events.some((event) => event.event === "transport_closed"), true)
  assert.equal(events.some((event) => event.event === "transport_resumed"), true)
})

test("LocalIpcClient keeps control requests alive when the event stream closes", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  server.on("connection", (socket) => {
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      if (frame.type === "subscribe") {
        socket.send(JSON.stringify({
          type: "response",
          request_id: frame.request_id,
          response: { ok: true },
          error: null,
        }))
        socket.send(JSON.stringify({
          type: "event",
          event_id: 1,
          event: { event: "heartbeat", session_id: frame.session_id },
        }))
        setTimeout(() => socket.close(1008, "event stream overloaded"), 10)
        return
      }

      if (frame.type === "request") {
        setTimeout(() => {
          socket.send(JSON.stringify({
            type: "response",
            request_id: frame.request_id,
            response: { ok: true, echoed: frame.request },
            error: null,
          }))
        }, 50)
      }
    })
  })

  const client = new LocalIpcClient(endpoint)
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  const responsePromise = client.send<{ ok: boolean; echoed: unknown }>({ op: "state" })
  await new Promise((resolve) => setTimeout(resolve, 75))
  const response = await responsePromise

  assert.equal(response.ok, true)
  assert.deepEqual(response.echoed, { op: "state" })
  assert.equal(events.some((event) => event.event === "transport_closed"), true)
})

test("LocalIpcClient does not reconnect only because subscribed app events are quiet", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const subscribeFrames: Array<Record<string, unknown>> = []
  server.on("connection", (socket) => {
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      if (frame.type !== "subscribe") {
        return
      }
      subscribeFrames.push(frame)
      socket.send(JSON.stringify({
        type: "response",
        request_id: frame.request_id,
        response: { ok: true },
        error: null,
      }))
      socket.send(JSON.stringify({
        type: "event",
        event_id: 1,
        event: {
          event: "heartbeat",
          session_id: frame.session_id,
        },
      }))
    })
  })

  const client = new LocalIpcClient(endpoint, { kernelPingIntervalMs: 1_000 })
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  await new Promise((resolve) => setTimeout(resolve, 350))

  assert.equal(subscribeFrames.length, 1)
  assert.equal(events.some((event) => event.event === "transport_closed"), false)
  assert.equal(events.some((event) => event.event === "transport_resumed"), false)
})

test("LocalIpcClient can opt into reconnecting when the subscribed event stream stalls", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const subscribeFrames: Array<Record<string, unknown>> = []
  let connectionCount = 0
  server.on("connection", (socket) => {
    connectionCount += 1
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      if (frame.type !== "subscribe") {
        return
      }
      subscribeFrames.push(frame)
      socket.send(JSON.stringify({
        type: "response",
        request_id: frame.request_id,
        response: { ok: true },
        error: null,
      }))
      socket.send(JSON.stringify({
        type: "event",
        event_id: connectionCount,
        event: {
          event: "heartbeat",
          session_id: frame.session_id,
        },
      }))
    })
  })

  const client = new LocalIpcClient(endpoint, { kernelEventStaleMs: 250 })
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => {
    dispose()
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  await new Promise((resolve) => setTimeout(resolve, 650))

  assert.equal(subscribeFrames.length >= 2, true)
  assert.equal(subscribeFrames[0]?.resume_from_event_id, null)
  assert.equal(subscribeFrames[1]?.resume_from_event_id, 1)
  assert.equal(events.some((event) => event.event === "transport_closed"), true)
  assert.equal(events.some((event) => event.event === "transport_resumed"), true)
})


test("LocalIpcClient uses relay request frames when relay mode is configured", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const receivedFrames: Array<Record<string, unknown>> = []
  server.on("connection", (socket) => {
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      receivedFrames.push(frame)

      if (frame.kind === "client_connect") {
        const daemon = createECDH("prime256v1")
        const daemonPublicKey = daemon.generateKeys().toString("base64")
        ;(socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon = daemon
        socket.send(JSON.stringify({
          kind: "client_connected",
          target: frame.target,
          daemon_public_key: daemonPublicKey,
        }))
        return
      }

      if (frame.kind === "client_request") {
        const daemon = (socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon
        const encryptedRequest = frame.encrypted_request as TestEncryptedRelayPayload
        const request = JSON.parse(decryptRelayPayload(daemon, encryptedRequest))
        socket.send(JSON.stringify({
          kind: "client_response",
          request_id: frame.request_id,
          encrypted_response: encryptRelayPayload(
            daemon,
            encryptedRequest.sender_public_key,
            Buffer.from(JSON.stringify({ ok: true, echoed: request }), "utf8"),
          ),
          error: null,
        }))
      }
    })
  })

  const client = new LocalIpcClient(endpoint, {
    relayAuthToken: "secret",
    targetDaemonId: "daemon-1",
  })
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  assert.equal(client.supportsKernelEvents(), true)
  const response = await client.send<{ ok: boolean; echoed: unknown }>({
    hello: "relay",
  })
  assert.equal(response.ok, true)
  assert.deepEqual(response.echoed, { hello: "relay" })
  assert.equal(receivedFrames[0]?.kind, "client_connect")
  assert.equal(receivedFrames[1]?.kind, "client_request")
  assert.equal(receivedFrames[0]?.auth_token, "secret")
  assert.deepEqual(receivedFrames[0]?.target, { daemon_id: "daemon-1", daemon_alias: null })
  assert.equal(typeof (receivedFrames[1]?.encrypted_request as TestEncryptedRelayPayload | undefined)?.ciphertext, "string")
  assert.equal(receivedFrames[1]?.request, undefined)
})

test("LocalIpcClient subscribes to relay kernel events with encrypted payloads", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const receivedFrames: Array<Record<string, unknown>> = []
  server.on("connection", (socket) => {
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      receivedFrames.push(frame)

      if (frame.kind === "client_connect") {
        const daemon = createECDH("prime256v1")
        const daemonPublicKey = daemon.generateKeys().toString("base64")
        ;(socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon = daemon
        socket.send(JSON.stringify({
          kind: "client_connected",
          target: frame.target,
          daemon_public_key: daemonPublicKey,
        }))
        return
      }

      if (frame.kind === "client_subscribe") {
        const daemon = (socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon
        const clientPublicKey = String(frame.client_public_key)
        socket.send(JSON.stringify({
          kind: "client_response",
          request_id: frame.request_id,
          encrypted_response: encryptRelayPayload(
            daemon,
            clientPublicKey,
            Buffer.from(JSON.stringify({ ok: true, resumed_from_event_id: frame.resume_from_event_id }), "utf8"),
          ),
          error: null,
        }))
        socket.send(JSON.stringify({
          kind: "client_event",
          subscription_id: frame.subscription_id,
          event_id: 1,
          encrypted_event: encryptRelayPayload(
            daemon,
            clientPublicKey,
            Buffer.from(JSON.stringify({
              event: "session_snapshot",
              session: { id: frame.session_id, attachment_ids: [frame.attachment_id] },
              provider_run: null,
            }), "utf8"),
          ),
        }))
        return
      }

      if (frame.kind === "client_unsubscribe") {
        const daemon = (socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon
        const clientPublicKey = String(frame.client_public_key)
        socket.send(JSON.stringify({
          kind: "client_response",
          request_id: frame.request_id,
          encrypted_response: encryptRelayPayload(
            daemon,
            clientPublicKey,
            Buffer.from(JSON.stringify({ ok: true }), "utf8"),
          ),
          error: null,
        }))
      }
    })
  })

  const client = new LocalIpcClient(endpoint, {
    relayAuthToken: "secret",
    targetDaemonId: "daemon-1",
  })
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => dispose())
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  await new Promise((resolve) => setTimeout(resolve, 25))
  await client.unsubscribeFromKernelEvents()

  assert.equal(receivedFrames[1]?.kind, "client_subscribe")
  assert.equal(receivedFrames[2]?.kind, "client_unsubscribe")
  assert.equal(typeof receivedFrames[1]?.client_public_key, "string")
  assert.deepEqual(events, [{
    event: "session_snapshot",
    session: { id: "session-1", attachment_ids: ["attachment-1"] },
    provider_run: null,
  }])
})

test("LocalIpcClient reconnects and resubscribes to relay events with the last received event id", async (t) => {
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")

  const address = server.address() as AddressInfo
  const endpoint = `ws://127.0.0.1:${address.port}`

  const subscribeFrames: Array<Record<string, unknown>> = []
  let connectionCount = 0
  server.on("connection", (socket) => {
    connectionCount += 1
    socket.on("message", (payload) => {
      const frame = JSON.parse(String(payload)) as Record<string, unknown>
      if (frame.kind === "client_connect") {
        const daemon = createECDH("prime256v1")
        const daemonPublicKey = daemon.generateKeys().toString("base64")
        ;(socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon = daemon
        socket.send(JSON.stringify({
          kind: "client_connected",
          target: frame.target,
          daemon_public_key: daemonPublicKey,
        }))
        return
      }
      if (frame.kind === "client_subscribe") {
        subscribeFrames.push(frame)
        const daemon = (socket as unknown as { daemon: ReturnType<typeof createECDH> }).daemon
        const clientPublicKey = String(frame.client_public_key)
        socket.send(JSON.stringify({
          kind: "client_response",
          request_id: frame.request_id,
          encrypted_response: encryptRelayPayload(
            daemon,
            clientPublicKey,
            Buffer.from(JSON.stringify({ ok: true, resumed_from_event_id: frame.resume_from_event_id }), "utf8"),
          ),
          error: null,
        }))
        if (connectionCount === 1) {
          socket.send(JSON.stringify({
            kind: "client_event",
            subscription_id: frame.subscription_id,
            event_id: 1,
            encrypted_event: encryptRelayPayload(
              daemon,
              clientPublicKey,
              Buffer.from(JSON.stringify({ event: "heartbeat", session_id: frame.session_id }), "utf8"),
            ),
          }))
          setTimeout(() => socket.close(), 10)
        } else {
          socket.send(JSON.stringify({
            kind: "client_event",
            subscription_id: frame.subscription_id,
            event_id: 2,
            encrypted_event: encryptRelayPayload(
              daemon,
              clientPublicKey,
              Buffer.from(JSON.stringify({
                event: "transport_resumed",
                session_id: frame.session_id,
                resumed_from_event_id: frame.resume_from_event_id,
              }), "utf8"),
            ),
          }))
        }
      }
    })
  })

  const client = new LocalIpcClient(endpoint, {
    relayAuthToken: "secret",
    targetDaemonId: "daemon-1",
  })
  const events: KernelEvent[] = []
  const dispose = client.onKernelEvent((event) => {
    events.push(event)
  })
  t.after(() => dispose())
  t.after(async () => {
    await client.close()
    await new Promise<void>((resolve) => {
      server.close(() => resolve())
    })
  })

  await client.subscribeToKernelEvents("session-1", "attachment-1")
  await new Promise((resolve) => setTimeout(resolve, 650))

  assert.equal(subscribeFrames.length >= 2, true)
  assert.equal(subscribeFrames[0]?.resume_from_event_id, null)
  assert.equal(subscribeFrames[1]?.resume_from_event_id, 1)
  assert.equal(events.some((event) => event.event === "transport_closed"), true)
  assert.equal(events.some((event) => event.event === "transport_resumed"), true)
})

const RELAY_NONCE_LEN = 12
const RELAY_TAG_LEN = 16
const RELAY_INFO = Buffer.from("arroba-relay-v1", "utf8")

function encryptRelayPayload(
  sender: ReturnType<typeof createECDH>,
  peerPublicKeyBase64: string,
  plaintext: Buffer,
) {
  const sharedSecret = sender.computeSecret(Buffer.from(peerPublicKeyBase64, "base64"))
  const key = Buffer.from(hkdfSync("sha256", sharedSecret, Buffer.alloc(0), RELAY_INFO, 32))
  const nonce = randomBytes(RELAY_NONCE_LEN)
  const cipher = createCipheriv("aes-256-gcm", key, nonce)
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final(), cipher.getAuthTag()])
  return {
    sender_public_key: sender.getPublicKey().toString("base64"),
    nonce: nonce.toString("base64"),
    ciphertext: ciphertext.toString("base64"),
  }
}

function decryptRelayPayload(
  receiver: ReturnType<typeof createECDH>,
  payload: { sender_public_key: string; nonce: string; ciphertext: string },
) {
  const sharedSecret = receiver.computeSecret(Buffer.from(payload.sender_public_key, "base64"))
  const key = Buffer.from(hkdfSync("sha256", sharedSecret, Buffer.alloc(0), RELAY_INFO, 32))
  const nonce = Buffer.from(payload.nonce, "base64")
  assert.equal(nonce.length, RELAY_NONCE_LEN)
  const ciphertext = Buffer.from(payload.ciphertext, "base64")
  assert.equal(ciphertext.length >= RELAY_TAG_LEN, true)
  const body = ciphertext.subarray(0, ciphertext.length - RELAY_TAG_LEN)
  const tag = ciphertext.subarray(ciphertext.length - RELAY_TAG_LEN)
  const decipher = createDecipheriv("aes-256-gcm", key, nonce)
  decipher.setAuthTag(tag)
  return Buffer.concat([decipher.update(body), decipher.final()]).toString("utf8")
}

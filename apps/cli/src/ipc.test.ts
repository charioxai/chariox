import test from "node:test"
import assert from "node:assert/strict"
import type { AddressInfo } from "node:net"
import { once } from "node:events"

import { WebSocketServer } from "ws"

import { LocalIpcClient, type KernelEvent } from "./ipc.js"

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

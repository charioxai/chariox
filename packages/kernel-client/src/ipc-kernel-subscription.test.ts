import assert from "node:assert/strict"
import { createServer } from "node:http"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, type LocalIpcError } from "./ipc.js"
import type { KernelEvent } from "./kernel-events.js"

test("a kernel subscription routes, resumes from the given event and surfaces frames and a later loss", async () => {
  const root = await mkdtemp(join(tmpdir(), "chx-unix-"))
  await mkdir(join(root, "run"), { mode: 0o700 })
  const path = join(root, "run", "k.sock")
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  const frames: Record<string, unknown>[] = []
  websocket.on("connection", (socket) => {
    socket.on("message", (bytes) => {
      const frame = JSON.parse(String(bytes))
      frames.push(frame)
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, kernel: "b3", response: { ok: true }, error: null }))
      socket.send(JSON.stringify({ type: "event", event_id: 8, kernel: "b3", event: { event: "client_catalog_changed", revision: 1 } }))
      socket.send(JSON.stringify({
        type: "response", request_id: frame.request_id, kernel: "b3", response: null,
        error: { code: "kernel_connection_lost", message: "Subscribe again with resume_from_event_id 8.", retryable: true },
      }))
    })
  })
  await new Promise<void>(resolve => server.listen(path, resolve))
  const ended: LocalIpcError[] = []
  const client = new LocalIpcClient(`ws+unix://${path}`, { kernel: "b3", onSubscriptionEnded: error => { ended.push(error) } })
  const seen: Array<[KernelEvent, unknown]> = []
  client.onKernelEvent((event, frame) => { seen.push([event, frame]) })
  try {
    await client.subscribeToKernelEvents("s-1", "att-1", 7)
    await new Promise<void>(resolve => setTimeout(resolve, 100))
    assert.equal(frames.length, 1)
    assert.deepEqual([frames[0]!.type, frames[0]!.kernel, frames[0]!.resume_from_event_id], ["subscribe", "b3", 7])
    assert.deepEqual(seen, [[{ event: "client_catalog_changed", revision: 1 }, { type: "event", event_id: 8, kernel: "b3", event: { event: "client_catalog_changed", revision: 1 } }]])
    assert.deepEqual(ended.map(error => error.code), ["kernel_connection_lost"])
    assert.match(ended[0]!.message, /Subscribe again with resume_from_event_id 8\./)
  } finally {
    await client.close()
    for (const socket of websocket.clients) socket.terminate()
    await new Promise<void>(resolve => websocket.close(() => resolve()))
    await new Promise<void>(resolve => server.close(() => resolve()))
    await rm(root, { recursive: true, force: true })
  }
})

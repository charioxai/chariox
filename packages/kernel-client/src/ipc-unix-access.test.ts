import assert from "node:assert/strict"
import { createServer } from "node:http"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"

for (const explicit of [true, false]) test(`${explicit ? "ws+unix" : "bare Unix path"} serves kernel websocket frames and never sends a bearer`, async () => {
  const root = await mkdtemp(join(tmpdir(), "chx-unix-"))
  await mkdir(join(root, "run"), { mode: 0o700 })
  const path = join(root, "run", "k.sock")
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  websocket.on("connection", (socket, request) => {
    assert.equal(request.headers.authorization, undefined)
    assert.equal(request.headers.origin, undefined)
    socket.on("message", (bytes) => {
      const frame = JSON.parse(String(bytes))
      assert.equal(frame.type, "request")
      assert.deepEqual(frame.request, { RequestKernelAccess: { holder_pid: process.pid } })
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: { KernelAccessGranted: { grant: { holder_pid: process.pid } } }, error: null }))
    })
  })
  await new Promise<void>(resolve => server.listen(path, resolve))
  const client = new LocalIpcClient(explicit ? `ws+unix://${path}` : path)
  try {
    assert.equal(client.supportsKernelEvents(), true)
    const result = await client.send<{ KernelAccessGranted: { grant: { holder_pid: number } } }>({ RequestKernelAccess: { holder_pid: process.pid } })
    assert.equal(result.KernelAccessGranted.grant.holder_pid, process.pid)
    assert.throws(() => new LocalIpcClient(`ws+unix://${path}`, { localAuthToken: "never" }))
  } finally {
    await client.close()
    for (const socket of websocket.clients) socket.terminate()
    await new Promise<void>(resolve => websocket.close(() => resolve()))
    await new Promise<void>(resolve => server.close(() => resolve()))
    await rm(root, { recursive: true, force: true })
  }
})

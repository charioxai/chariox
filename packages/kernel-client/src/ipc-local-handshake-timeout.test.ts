import assert from "node:assert/strict"
import { createServer, type Socket } from "node:net"
import test from "node:test"

import type WebSocket from "ws"

import { LocalIpcClient } from "./ipc.js"

// MP-08 / MP-09 / MP-10 / MP-11 (W1): a kernel that completes the TCP connection but never
// accepts it must not leave a local client waiting for ever.
test("a loopback upgrade the kernel never answers fails instead of waiting", async (t) => {
  const held: Socket[] = []
  const server = createServer((socket) => held.push(socket))
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve))
  t.after(async () => {
    held.forEach((socket) => socket.destroy())
    await new Promise<void>((resolve) => server.close(() => resolve()))
  })
  const address = server.address()
  assert.ok(address && typeof address !== "string")
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`)
  t.after(() => client.destroy())
  const internals = client as unknown as { localHandshakeTimeoutMs: number; openKernelWebSocket: () => WebSocket }
  internals.localHandshakeTimeoutMs = 200

  const socket = internals.openKernelWebSocket()
  const failure = await new Promise<Error>((resolve, reject) => {
    const guard = setTimeout(() => reject(new Error("the upgrade was still pending after 5 s")), 5_000)
    socket.once("error", (error) => {
      clearTimeout(guard)
      resolve(error)
    })
  })
  assert.match(failure.message, /handshake has timed out/i)
})

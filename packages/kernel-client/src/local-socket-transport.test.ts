import assert from "node:assert/strict"
import net from "node:net"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { once } from "node:events"

import { LocalIpcError } from "./local-ipc-error.js"
import { sendLocalSocketRequest } from "./local-socket-transport.js"

test("sendLocalSocketRequest writes framed JSON and decodes response envelopes", async (t) => {
  const { server, socketPath, received } = await startLocalSocketServer((request) => ({
    response: { ok: true, echoed: request },
    error: null,
  }))
  t.after(async () => {
    await closeLocalSocketServer(server, socketPath)
  })

  const response = await sendLocalSocketRequest<{ ok: boolean; echoed: unknown }>(
    socketPath,
    { ListSessions: null },
    1_000,
  )

  assert.deepEqual(received, [{ ListSessions: null }])
  assert.deepEqual(response, {
    ok: true,
    echoed: { ListSessions: null },
  })
})

test("sendLocalSocketRequest converts error envelopes to LocalIpcError", async (t) => {
  const { server, socketPath } = await startLocalSocketServer(() => ({
    response: null,
    error: "denied",
  }))
  t.after(async () => {
    await closeLocalSocketServer(server, socketPath)
  })

  await assert.rejects(
    () => sendLocalSocketRequest(socketPath, { DeleteKernel: null }, 1_000),
    (error) => error instanceof LocalIpcError
      && error.operation === "handle local response"
      && error.message.includes("denied"),
  )
})

async function startLocalSocketServer(
  respond: (request: unknown) => { response: unknown; error: string | null },
) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-kernel-client-"))
  const socketPath = path.join(dir, "kernel.sock")
  const received: unknown[] = []
  const server = net.createServer((socket) => {
    const chunks: Buffer[] = []
    socket.on("data", (chunk) => {
      chunks.push(chunk)
      const request = readFramedJson(Buffer.concat(chunks))
      if (request === null) {
        return
      }
      received.push(request)
      const response = Buffer.from(JSON.stringify(respond(request)), "utf8")
      const frame = Buffer.allocUnsafe(4 + response.length)
      frame.writeUInt32BE(response.length, 0)
      response.copy(frame, 4)
      socket.end(frame)
    })
  })
  server.listen(socketPath)
  await once(server, "listening")
  return { server, socketPath, received }
}

async function closeLocalSocketServer(server: net.Server, socketPath: string) {
  await new Promise<void>((resolve) => {
    server.close(() => resolve())
  })
  await rm(path.dirname(socketPath), { recursive: true, force: true })
}

function readFramedJson(frame: Buffer): unknown | null {
  if (frame.length < 4) {
    return null
  }
  const length = frame.readUInt32BE(0)
  if (frame.length < 4 + length) {
    return null
  }
  return JSON.parse(frame.subarray(4, 4 + length).toString("utf8"))
}

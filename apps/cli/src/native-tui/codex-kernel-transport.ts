// MP-08 / MP-10 / MP-11: official Codex frames reach the owning kernel;
// worker loopback URLs never become client connection targets.
import { once } from "node:events"
import { setTimeout as sleep } from "node:timers/promises"
import WebSocket, { WebSocketServer } from "ws"
import type { LocalIpcClient } from "../ipc.js"
import type { NativeProviderTransportOperation, NativeProviderTransportResult } from "@chariox/kernel-client/kernel-types"
import { nativeProviderTransportRequest } from "../ipc-requests.js"

const frameBytes = 256 * 1024
const queueFrames = 32

export async function startCodexKernelTransport(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  providerRunId: string
}): Promise<{ endpoint: string; close: () => Promise<void> }> {
  let closed = false
  const tasks = new Set<Promise<void>>()
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0, maxPayload: frameBytes })
  await once(server, "listening")
  const address = server.address()
  if (!address || typeof address === "string") throw new Error("Codex kernel bridge did not bind")
  const send = async (operation: NativeProviderTransportOperation) => {
    const response = await options.client.send(nativeProviderTransportRequest({ ...options, operation })) as { NativeProviderTransport?: { result: NativeProviderTransportResult } }
    const result = response.NativeProviderTransport?.result
    if (!result) throw new Error("Codex native transport requires kernel protocol 526 and worker protocol 134; update and reattach")
    return result
  }
  server.on("connection", socket => {
    if (closed || server.clients.size > 2) { socket.close(1013, "native connection limit"); return }
    const frames: string[] = []
    let bytes = 0
    let finished = false
    let connectionId: string | undefined
    socket.on("close", () => { finished = true })
    socket.on("error", () => { finished = true })
    socket.on("message", (raw, binary) => {
      if (binary || frames.length >= queueFrames || bytes + Buffer.byteLength(raw.toString()) > frameBytes) {
        finished = true
        socket.close(1009, "native frame queue exceeded")
        return
      }
      const frame = raw.toString()
      frames.push(frame)
      bytes += Buffer.byteLength(frame)
    })
    const task = (async () => {
      try {
        const opened = await send({ kind: "open" })
        connectionId = opened.connection_id
        while (!closed && !finished) {
          const outbound = frames.splice(0)
          bytes = 0
          const result = await send({ kind: "exchange", connection_id: connectionId, frames: outbound })
          if (result.connection_id !== connectionId) throw new Error("native connection identity changed")
          for (const frame of result.frames) {
            if (socket.readyState === WebSocket.OPEN) {
              if (socket.bufferedAmount + Buffer.byteLength(frame) > frameBytes * queueFrames) { finished = true; socket.close(1013, "native output backpressure"); break }
              socket.send(frame)
            }
          }
          if (result.closed) break
          if (result.frames.length === 0 && frames.length === 0) await sleep(25)
        }
      } catch (error) {
        if (socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify({ method: "error", params: { message: error instanceof Error ? error.message : "Codex kernel transport failed" } }))
        }
      } finally {
        if (connectionId) await send({ kind: "close", connection_id: connectionId }).catch(() => {})
        socket.close()
      }
    })()
    tasks.add(task)
    void task.finally(() => tasks.delete(task))
  })
  return {
    endpoint: `ws://127.0.0.1:${address.port}`,
    close: async () => {
      closed = true
      for (const socket of server.clients) socket.close()
      await Promise.allSettled([...tasks])
      await new Promise<void>(resolve => server.close(() => resolve()))
    },
  }
}

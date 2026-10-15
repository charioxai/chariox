// MP-08 / MP-10 / MP-11: official Codex frames reach the owning kernel;
// worker loopback URLs never become client connection targets.
import { once } from "node:events"
import { randomUUID } from "node:crypto"
import { setTimeout as sleep } from "node:timers/promises"
import WebSocket, { WebSocketServer } from "ws"
import { LocalIpcError, type LocalIpcClient } from "../ipc.js"
import type { NativeProviderTransportOperation, NativeProviderTransportResult } from "@chariox/kernel-client/kernel-types"
import { nativeProviderTransportRequest, nativeProviderTransportMinimumProtocolVersion } from "../ipc-requests.js"

import { traceCodexLifecycle } from "./codex-lifecycle-trace.js"

const frameBytes = 256 * 1024
const outputFrameBytes = 16 * 1024 * 1024
const outputBufferBytes = 32 * 1024 * 1024
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
  const sendOnce = async (operation: NativeProviderTransportOperation) => {
    let response: { NativeProviderTransport?: { result: NativeProviderTransportResult } }
    try {
      response = await options.client.send(nativeProviderTransportRequest({ ...options, operation }))
    } catch (error) {
      traceCodexLifecycle("native_transport_error", {
        method: operation.kind,
        errorClass: error instanceof Error ? error.name : typeof error,
        errorCode: typeof (error as { code?: unknown })?.code === "string"
          && /^[a-z_]{1,64}$/.test((error as { code: string }).code) ? (error as { code: string }).code : undefined,
        reasonClass: error instanceof Error ? ["native connection closed", "native attachment", "native projected run", "native lease", "not configured", "not connected", "kernel may have run", "timed out"].find(reason => error.message.includes(reason)) : undefined,
      })
      if (error instanceof Error && /unknown variant.*NativeProviderTransport|unsupported.*native.?provider.?transport/i.test(error.message)) {
        throw new Error(`Codex native transport requires kernel protocol ${nativeProviderTransportMinimumProtocolVersion} and the matching worker protocol; update the owning kernel and reattach`)
      }
      throw error
    }
    if (!response.NativeProviderTransport) traceCodexLifecycle("native_transport_refused", { method: operation.kind, responseKinds: Object.keys(response) })
    const result = response.NativeProviderTransport?.result
    if (!result || result.sequence !== (operation.kind === "exchange" ? operation.sequence : 0)) throw new Error(`Codex native transport requires kernel protocol ${nativeProviderTransportMinimumProtocolVersion} and the matching worker protocol; update and reattach`)
    return result
  }
  const send = async (operation: NativeProviderTransportOperation) => {
    const deadline = Date.now() + 15_000
    for (;;) {
      try { return await sendOnce(operation) }
      catch (error) {
        // These operations carry worker-owned receipts. Retry this exact
        // operation, including its sequence and input, never a new exchange.
        if (closed || operation.kind === "close" || Date.now() >= deadline
          || !(error instanceof LocalIpcError)
          || !["target_disconnected", "target_not_connected", "connection_closed", "request_timeout", "outcome_unknown", "write_failed"].includes(error.code ?? "")) throw error
        await sleep(100)
      }
    }
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
        const opened = await send({ kind: "open", request_id: randomUUID() })
        connectionId = opened.connection_id
        let sequence = 1
        while (!closed && !finished) {
          const outbound = frames.splice(0)
          bytes = 0
          const result = await send({ kind: "exchange", connection_id: connectionId, frames: outbound, sequence })
          if (result.connection_id !== connectionId) throw new Error("native connection identity changed")
          sequence += 1
          for (const frame of result.frames) {
            if (socket.readyState === WebSocket.OPEN) {
              if (Buffer.byteLength(frame) > outputFrameBytes || socket.bufferedAmount + Buffer.byteLength(frame) > outputBufferBytes) { finished = true; socket.close(1013, "native output backpressure"); break }
              socket.send(frame)
            }
          }
          if (result.closed) { traceCodexLifecycle("native_transport_closed", { method: "exchange" }); break }
          if (result.frames.length === 0 && frames.length === 0) await sleep(25)
        }
      } catch (error) {
        if (error instanceof Error && /^Codex native transport requires kernel protocol /.test(error.message)) {
          process.stderr.write(`${error.message}\n`)
        }
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

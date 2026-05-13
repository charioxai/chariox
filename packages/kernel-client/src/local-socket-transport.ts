import net from "node:net"

import type { IpcEnvelope } from "./kernel-transport-frames.js"
import { LocalIpcError } from "./local-ipc-error.js"

export function sendLocalSocketRequest<TResponse>(
  socketPath: string,
  request: unknown,
  timeoutMs: number,
): Promise<TResponse> {
  return new Promise<TResponse>((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    const chunks: Buffer[] = []
    let settled = false

    const fail = (operation: string, error: unknown) => {
      if (settled) {
        return
      }
      settled = true
      socket.destroy()
      reject(new LocalIpcError(operation, error instanceof Error ? error.message : String(error)))
    }

    const succeed = (value: TResponse) => {
      if (settled) {
        return
      }
      settled = true
      socket.destroy()
      resolve(value)
    }

    socket.setTimeout(timeoutMs)
    socket.once("timeout", () => fail("handle local response", "timed out"))
    socket.once("error", (error) => fail("connect local socket", error))

    socket.once("connect", () => {
      let payload: Buffer
      try {
        payload = Buffer.from(JSON.stringify(request), "utf8")
      } catch (error) {
        fail("serialize local request", error)
        return
      }

      const frame = Buffer.allocUnsafe(4 + payload.length)
      frame.writeUInt32BE(payload.length, 0)
      payload.copy(frame, 4)

      socket.write(frame, (error) => {
        if (error) {
          fail("write local request", error)
        }
      })
    })

    socket.on("data", (chunk) => {
      chunks.push(chunk)
    })

    socket.once("end", () => {
      const buffer = Buffer.concat(chunks)
      if (buffer.length < 4) {
        fail("read local response header", "response header was truncated")
        return
      }

      const payloadLength = buffer.readUInt32BE(0)
      const payload = buffer.subarray(4)
      if (payload.length < payloadLength) {
        fail("read local response body", "response body was truncated")
        return
      }

      let envelope: IpcEnvelope<TResponse>
      try {
        envelope = JSON.parse(payload.subarray(0, payloadLength).toString("utf8")) as IpcEnvelope<TResponse>
      } catch (error) {
        fail("decode local response", error)
        return
      }

      if (envelope.error) {
        fail("handle local response", envelope.error)
        return
      }
      if (envelope.response == null) {
        fail("handle local response", "response envelope was empty")
        return
      }

      succeed(envelope.response)
    })
  })
}

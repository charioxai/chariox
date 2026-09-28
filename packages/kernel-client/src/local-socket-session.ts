import net from "node:net"
import { LocalIpcError } from "./local-ipc-error.js"
import { requireKernelControlCapability } from "./ipc-disposable-worker-requests.js"

export const guardedControlSessionRequest = { GuardedControlSession: { version: 1 } } as const
const maxFrameBytes = 1024 * 1024

// A single connection owns admission and execution. Never reconnect or replay a command.
export function sendGuardedLocalSocketRequest<T>(path: string, request: unknown, timeoutMs: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(path)
    let buffer = Buffer.alloc(0)
    let phase: "probe" | "validating" | "command" | "done" = "probe"
    const timer = setTimeout(() => fail("timed out"), timeoutMs)
    function fail(error: unknown) {
      if (phase === "done") return
      phase = "done"
      clearTimeout(timer)
      socket.destroy()
      reject(new LocalIpcError("guarded local control", error instanceof Error ? error.message : String(error)))
    }
    function write(value: unknown) {
      if (socket.destroyed || socket.readableEnded || socket.writableEnded) throw new Error("admitted connection closed")
      const payload = Buffer.from(JSON.stringify(value))
      if (payload.length > maxFrameBytes) throw new Error("request exceeded frame limit")
      const frame = Buffer.alloc(4 + payload.length)
      frame.writeUInt32BE(payload.length)
      payload.copy(frame, 4)
      socket.write(frame, error => { if (error) fail(error) })
    }
    socket.once("connect", () => { try { write(guardedControlSessionRequest) } catch (error) { fail(error) } })
    socket.once("error", fail)
    socket.once("end", () => fail("admitted connection closed before response"))
    socket.once("close", () => fail("admitted connection closed before response"))
    socket.on("data", chunk => {
      try {
        if (phase === "done") return
        buffer = Buffer.concat([buffer, chunk])
        if (buffer.length < 4) return
        const size = buffer.readUInt32BE(0)
        if (size > maxFrameBytes || buffer.length > size + 4) throw new Error("invalid response frame size")
        if (buffer.length < size + 4) return
        const envelope = JSON.parse(buffer.subarray(4).toString("utf8"))
        buffer = Buffer.alloc(0)
        if (envelope.error) throw new Error(phase === "probe"
          ? "kernel does not support guarded Unix control sessions; protocol 367 or newer is required"
          : envelope.error)
        if (envelope.response == null) throw new Error("empty response")
        if (phase === "probe") {
          if (envelope.session?.version !== 1) throw new Error("kernel does not support guarded Unix control sessions")
          phase = "validating"
          void requireKernelControlCapability(async () => envelope.response, request).then(() => {
            if (phase !== "validating") return
            try { phase = "command"; write(request) } catch (error) { fail(error) }
          }, fail)
        } else if (phase === "command") {
          phase = "done"
          clearTimeout(timer)
          socket.destroy()
          resolve(envelope.response as T)
        } else throw new Error("unexpected response during admission")
      } catch (error) { fail(error) }
    })
  })
}

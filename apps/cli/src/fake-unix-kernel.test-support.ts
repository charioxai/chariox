import { createHash } from "node:crypto"
import { createServer } from "node:http"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { WebSocketServer, type WebSocket } from "ws"

export type FakeKernelFrame = { type: string; request_id: string; command_id: string; request: Record<string, unknown>; kernel?: string; resume_from_event_id?: number | null }

export type FakeKernel = { path: string; frames: FakeKernelFrame[]; close(): Promise<void> }

export function replyWith(socket: WebSocket, frame: FakeKernelFrame, outcome: { response: unknown } | { error: { code: string; message: string; retryable: boolean } }): void {
  socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response: null, error: null, ...outcome }))
}

export async function startFakeKernel(onRequest: (socket: WebSocket, frame: FakeKernelFrame, attempt: number) => void): Promise<FakeKernel> {
  const root = await mkdtemp(join(tmpdir(), "chx-fake-kernel-"))
  await mkdir(join(root, "run"), { mode: 0o700 })
  const path = join(root, "run", "k.sock")
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  const frames: FakeKernelFrame[] = []
  websocket.on("connection", socket => socket.on("message", bytes => {
    const frame = JSON.parse(String(bytes)) as FakeKernelFrame
    frames.push(frame)
    onRequest(socket, frame, frames.length)
  }))
  await new Promise<void>(resolve => server.listen(path, resolve))
  return {
    path,
    frames,
    async close() {
      for (const socket of websocket.clients) socket.terminate()
      await new Promise<void>(resolve => websocket.close(() => resolve()))
      await new Promise<void>(resolve => server.close(() => resolve()))
      await rm(root, { recursive: true, force: true })
    },
  }
}

export type FakeKernelFiles = { files: Map<string, Buffer>; partials: Map<string, Buffer>; sockets: Set<WebSocket> }

export function fakeKernelFiles(): FakeKernelFiles {
  return { files: new Map(), partials: new Map(), sockets: new Set() }
}

const sha256 = (bytes: Buffer) => createHash("sha256").update(bytes).digest("hex")
const failure = (code: string, message: string) => ({ error: { code, message, retryable: false } })

// Serves the two kernel file requests from memory with the kernel's offset and hash rules.
export function serveKernelFiles(store: FakeKernelFiles): (socket: WebSocket, frame: FakeKernelFrame) => void {
  return (socket, frame) => {
    store.sockets.add(socket)
    const [name, body] = Object.entries(frame.request)[0] as [string, any]
    const file = store.files.get(body?.path)
    if (name === "RelayStatus") {
      replyWith(socket, frame, { response: { RelayStatus: { status: { daemon_id: "d", machine_id: "m", capabilities: ["kernel_gateway_v1"] } } } })
    } else if (name === "ReadKernelFile") {
      // An unfinished upload is readable under its staging name, like on the kernel.
      const read = file ?? (body.path.endsWith(".chariox-partial") ? store.partials.get(body.path.slice(0, -".chariox-partial".length)) : undefined)
      if (!read) return replyWith(socket, frame, failure("file_not_found", `${body.path} does not exist.`))
      const data = read.subarray(body.offset, body.offset + body.length)
      replyWith(socket, frame, { response: { KernelFileChunk: { path: body.path, offset: body.offset, data_base64: data.toString("base64"), size: read.length, eof: body.offset + data.length >= read.length } } })
    } else if (name === "WriteKernelFile") {
      if (file && !body.overwrite) return replyWith(socket, frame, failure("file_exists", `${body.path} already exists; send overwrite to replace it.`))
      const have = body.offset === 0 ? Buffer.alloc(0) : store.partials.get(body.path) ?? Buffer.alloc(0)
      if (have.length !== body.offset) return replyWith(socket, frame, failure("file_offset_mismatch", `${body.path} has ${have.length} bytes of this upload; continue at offset ${have.length}.`))
      const received = Buffer.concat([have, Buffer.from(body.data_base64, "base64")])
      const complete = received.length === body.total_size
      if (complete && body.sha256 && sha256(received) !== body.sha256) {
        store.partials.delete(body.path)
        return replyWith(socket, frame, failure("file_hash_mismatch", "The upload did not match its sha256, so nothing was kept."))
      }
      if (complete) { store.files.set(body.path, received); store.partials.delete(body.path) } else store.partials.set(body.path, received)
      replyWith(socket, frame, { response: { KernelFileWritten: { path: body.path, received: received.length, complete, sha256: complete ? sha256(received) : null } } })
    }
  }
}

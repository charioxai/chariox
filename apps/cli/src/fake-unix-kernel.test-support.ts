import { createServer } from "node:http"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { WebSocketServer, type WebSocket } from "ws"

export type FakeKernelFrame = { type: string; request_id: string; command_id: string; request: Record<string, unknown>; machine?: string }

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

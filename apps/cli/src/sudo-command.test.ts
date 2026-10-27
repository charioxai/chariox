import assert from "node:assert/strict"
import { test } from "node:test"
import { createServer } from "node:http"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { WebSocketServer } from "ws"
import { runSudoCommand, parseSudoRequest } from "./sudo-command.js"

test("external sudo sends the exact target and full prompt without credentials", () => {
  assert.deepEqual(parseSudoRequest(["--agent", "target", "--prompt", "first\nsecond", "--socket", "/tmp/k.sock"]), {
    request: { RequestKernelSudo: { agent_id: "target", prompt: "first\nsecond" } }, socket: "/tmp/k.sock",
  })
  for (const argv of [[], ["--agent", "target"], ["--agent", "target", "--prompt", " "], ["--agent", "a", "--agent", "b", "--prompt", "x"], ["--passkey", "never"]]) {
    assert.throws(() => parseSudoRequest(argv))
  }
})

test("sudo CLI uses the Unix kernel frames and returns only the outcome", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chx-sudo-cli-"))
  const path = join(root, "k.sock")
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  const output: unknown[][] = []
  t.mock.method(console, "log", (...args: unknown[]) => output.push(args))
  t.mock.method(console, "error", () => {})
  websocket.on("connection", (socket, request) => {
    assert.equal(request.headers.authorization, undefined)
    assert.equal(request.headers.origin, undefined)
    socket.on("message", (bytes) => {
      const frame = JSON.parse(String(bytes))
      assert.deepEqual(frame.request, { RequestKernelSudo: { agent_id: "target", prompt: "full\nprompt" } })
      socket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
        response: { KernelSudoRequested: { agent_id: "target" } }, error: null }))
    })
  })
  await new Promise<void>(resolve => server.listen(path, resolve))
  try {
    assert.equal(await runSudoCommand(["sudo", "request", "--agent", "target", "--prompt", "full\nprompt", "--socket", path]), true)
    assert.deepEqual(output, [[JSON.stringify({ agent_id: "target" })]])
    assert.equal(await runSudoCommand(["access", "list"]), false)
  } finally {
    for (const socket of websocket.clients) socket.terminate()
    await new Promise<void>(resolve => websocket.close(() => resolve()))
    await new Promise<void>(resolve => server.close(() => resolve()))
    await rm(root, { recursive: true, force: true })
  }
})

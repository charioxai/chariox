import assert from "node:assert/strict"
import { once } from "node:events"
import test from "node:test"
import WebSocket from "ws"
import type { LocalIpcClient } from "../ipc.js"
import { startCodexKernelTransport } from "./codex-kernel-transport.js"

test("MP-08 Codex upstream crosses the owning kernel API and closes its exact connection", async () => {
  const seen: Array<Record<string, unknown>> = []
  const client = { send: async (request: { NativeProviderTransport: Record<string, unknown> }) => {
    const scope = request.NativeProviderTransport
    seen.push(scope)
    const op = scope.operation as { kind: string; frames?: string[] }
    return { NativeProviderTransport: { result: { connection_id: "owning-connection", closed: op.kind === "close",
      frames: op.frames?.length ? [JSON.stringify({ id: 7, result: { userAgent: "Codex 0.159.3" } })] : [] } } }
  } } as unknown as LocalIpcClient
  const transport = await startCodexKernelTransport({ client, sessionId: "room", attachmentId: "attachment", agentId: "agent", providerRunId: "projected-run" })
  const socket = new WebSocket(transport.endpoint)
  try {
    await once(socket, "open")
    const reply = once(socket, "message")
    socket.send(JSON.stringify({ id: 7, method: "initialize", params: { clientInfo: { name: "codex_cli_rs", version: "0.159.3" } } }))
    const [frame] = await reply
    assert.equal(JSON.parse(frame.toString()).result.userAgent, "Codex 0.159.3")
    assert.ok(seen.some(scope => (scope.operation as { kind: string }).kind === "open"))
    assert.ok(seen.every(scope => scope.session_id === "room" && scope.attachment_id === "attachment" && scope.agent_id === "agent" && scope.provider_run_id === "projected-run" && !("endpoint" in scope)))
  } finally {
    socket.close()
    await transport.close()
  }
  assert.ok(seen.some(scope => (scope.operation as {kind: string; connection_id?: string}).kind === "close" && (scope.operation as {connection_id?: string}).connection_id === "owning-connection"))
})

import assert from "node:assert/strict"
import { once } from "node:events"
import test from "node:test"
import WebSocket from "ws"
import { LocalIpcError, type LocalIpcClient } from "../ipc.js"
import { startCodexKernelTransport } from "./codex-kernel-transport.js"

test("MP-08 Codex upstream crosses the owning kernel API and closes its exact connection", async () => {
  const seen: Array<Record<string, unknown>> = []
  const client = { send: async (request: { NativeProviderTransport: Record<string, unknown> }) => {
    const scope = request.NativeProviderTransport
    seen.push(scope)
    const op = scope.operation as { kind: string; sequence?: number; frames?: string[] }
    return { NativeProviderTransport: { result: { connection_id: "owning-connection", sequence: op.sequence ?? 0, closed: op.kind === "close",
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

// MP-08 / MP-10 / MP-11: the older kernel RPC refusal becomes visible upgrade guidance.
test("MP-08 old owning kernel gets an actionable scoped transport upgrade", async () => {
  const client = { send: async () => { throw new Error("unknown variant NativeProviderTransport") } } as unknown as LocalIpcClient
  const transport = await startCodexKernelTransport({ client, sessionId: "room", attachmentId: "attachment", agentId: "agent", providerRunId: "run" })
  const socket = new WebSocket(transport.endpoint)
  try {
    const reply = once(socket, "message")
    const [frame] = await reply
    assert.match(JSON.parse(frame.toString()).params.message, /kernel protocol 531 and the matching worker protocol; update the owning kernel and reattach/)
  } finally { socket.close(); await transport.close() }
})

for (const code of ["target_disconnected", "target_not_connected"]) test(`MP-08 ${code} retries the identical acknowledged operation`, async () => {
  let inputWrites = 0
  let lost = false
  let retained: { sequence: number; frames: string[] } | null = null
  const client = { send: async (request: { NativeProviderTransport: { operation: { kind: string; sequence?: number; frames?: string[] } } }) => {
    const op = request.NativeProviderTransport.operation
    if (op.frames?.length) {
      if (retained?.sequence !== op.sequence) {
        inputWrites += 1
        retained = { sequence: op.sequence!, frames: [JSON.stringify({ id: 17, result: { recovered: true } })] }
      }
      if (!lost) { lost = true; throw new LocalIpcError("exchange", "scoped relay gap", code, true) }
      assert.deepEqual(op.frames, [JSON.stringify({ id: 17, method: "model/list" })])
      return { NativeProviderTransport: { result: { connection_id: "owning", ...retained, closed: false } } }
    }
    return { NativeProviderTransport: { result: { connection_id: "owning", sequence: op.sequence ?? 0, frames: [], closed: op.kind === "close" } } }
  } } as unknown as LocalIpcClient
  const transport = await startCodexKernelTransport({ client, sessionId: "room", attachmentId: "view", agentId: "agent", providerRunId: "run" })
  const socket = new WebSocket(transport.endpoint)
  try {
    await once(socket, "open")
    const reply = once(socket, "message")
    socket.send(JSON.stringify({ id: 17, method: "model/list" }))
    assert.equal(JSON.parse((await reply)[0].toString()).result.recovered, true)
    assert.equal(inputWrites, 1)
    assert.equal(socket.readyState, WebSocket.OPEN)
  } finally { socket.close(); await transport.close() }
})

test("MP-08 lost open acknowledgement reuses one scoped request identity", async () => {
  const opens: string[] = []
  const client = { send: async (request: { NativeProviderTransport: { operation: { kind: string; request_id?: string; sequence?: number } } }) => {
    const op = request.NativeProviderTransport.operation
    if (op.kind === "open") {
      opens.push(op.request_id!)
      if (opens.length === 1) throw new LocalIpcError("open", "lost reply", "target_disconnected", true)
    }
    return { NativeProviderTransport: { result: { connection_id: "owning", sequence: op.sequence ?? 0, frames: [], closed: op.kind === "close" } } }
  } } as unknown as LocalIpcClient
  const transport = await startCodexKernelTransport({ client, sessionId: "room", attachmentId: "view", agentId: "agent", providerRunId: "run" })
  const socket = new WebSocket(transport.endpoint)
  try {
    await once(socket, "open")
    for (let n = 0; n < 100 && opens.length < 2; n++) await new Promise(resolve => setTimeout(resolve, 10))
    assert.equal(opens.length, 2)
    assert.equal(opens[0], opens[1])
    assert.match(opens[0]!, /^[0-9a-f-]{36}$/)
  } finally { socket.close(); await transport.close() }
})

test("MP-08 missing receipt acknowledgement refuses an older transport before frames", async () => {
  const operations: string[] = []
  const client = { send: async (request: { NativeProviderTransport: { operation: { kind: string } } }) => {
    const op = request.NativeProviderTransport.operation
    operations.push(op.kind)
    return { NativeProviderTransport: { result: { connection_id: "old", frames: [], closed: false } } }
  } } as unknown as LocalIpcClient
  const transport = await startCodexKernelTransport({ client, sessionId: "room", attachmentId: "view", agentId: "agent", providerRunId: "run" })
  const socket = new WebSocket(transport.endpoint)
  try {
    const reply = once(socket, "message")
    assert.match(JSON.parse((await reply)[0].toString()).params.message, /matching worker protocol; update and reattach/)
    assert.deepEqual(operations, ["open"])
  } finally { socket.close(); await transport.close() }
})

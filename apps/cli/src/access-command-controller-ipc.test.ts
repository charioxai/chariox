import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as delay } from "node:timers/promises"
import { WebSocketServer } from "ws"
import { LocalIpcClient } from "./ipc.js"
import { createAccessCommandController } from "./access-command-controller.js"
import { beginMutableLocalIpcClientPivot, createMutableLocalIpcClient } from "./mutable-local-ipc-client.js"

async function until(check: () => boolean) {
  const deadline = Date.now() + 3000
  while (!check()) {
    assert.ok(Date.now() < deadline, "expected transport activity")
    await delay(10)
  }
}
async function peer(t: test.TestContext, cursor: number, agentId: string) {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`)
  const requests: { request_id: string; command_id: string; request: { KernelBrowser: { command: { op: string; after?: number } } } }[] = []
  const polls: { after: number; wait_ms: number }[] = []
  const pending: (() => void)[] = []
  const timers = new Set<ReturnType<typeof setTimeout>>()
  let connections = 0, revokes = 0, lists = 0
  let notice: unknown = null
  const snapshot = () => ({ event: "user_domain_grants_changed", cursor,
    room_computer: [{ agent_id: agentId, session_id: "session", allowed: false }],
    grants: [{ agent_id: agentId, session_id: "session", kernel_id: agentId,
      resources: [{ kind: "note", note_id: "note" }], since_ms: 100,
      focused: false, idle_since_ms: null, idle_timeout_seconds: 1800 }], notice })
  let holdPolls = false
  server.on("connection", socket => {
    connections++
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload))
      requests.push(frame)
      const command = frame.request.KernelBrowser.command
      const reply = () => {
        if (socket.readyState === 1) socket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
          response: { KernelBrowser: { result: snapshot() } }, error: null }))
      }
      const later = (ms: number) => {
        const timer = setTimeout(() => { timers.delete(timer); reply() }, ms)
        timers.add(timer)
      }
      if (command.op === "subscribe_grants") {
        polls.push(command)
        // A real kernel responds immediately to a mismatched cursor.
        if (command.after !== cursor) reply()
        else if (holdPolls) pending.push(reply)
        else later(command.wait_ms)
      } else if (command.op === "revoke_grants") { revokes++; later(6000) }
      else { lists++; reply() }
    })
  })
  t.after(async () => {
    client.destroy()
    for (const timer of timers) clearTimeout(timer)
    for (const socket of server.clients) socket.terminate()
    await new Promise<void>(resolve => server.close(() => resolve()))
  })
  return { client, requests, polls, pending, get connections() { return connections }, get revokes() { return revokes },
    get lists() { return lists }, hold: () => { holdPolls = true },
    restart: () => {
      cursor = 1; notice = null; pending.length = 0
      for (const socket of server.clients) socket.terminate()
    },
    use: () => { cursor++; notice = { agent_id: agentId, resource: { kind: "note", note_id: "note" }, at_ms: 500 } } }
}

test("idle grant feed and overlapping revocation share the real default-timeout IPC connection", async t => {
  const kernel = await peer(t, 10, "holder")
  const lines: string[] = []
  const controller = createAccessCommandController({ client: createMutableLocalIpcClient(kernel.client), appendNotice: line => lines.push(line) })
  t.after(() => controller.stop())
  controller.start()
  await until(() => kernel.polls.length === 1)
  // This mutation remains pending beyond the observation watchdog's default 5s.
  await controller.handle(["revoke", "holder"])
  assert.match(lines.at(-1)!, /Revoked access and Room Computer control for holder\./)
  assert.match(lines.at(-1)!, /^holder · session session · Room Computer revoked$/m)
  assert.equal(kernel.revokes, 1)
  assert.equal(kernel.connections, 1, "idle observations must not reset the shared control socket")
  assert.ok(kernel.polls.length >= 5 && kernel.polls.length <= 8)
  assert.ok(kernel.polls.every(poll => poll.wait_ms === 1000 && poll.after === 10))
})

test("transactional real-IPC pivots reset a higher cursor and fence the retained source client's replies", async t => {
  const source = await peer(t, 100, "source"), target = await peer(t, 1, "target")
  source.hold(); target.hold()
  const facade = createMutableLocalIpcClient(source.client), lines: string[] = []
  const controller = createAccessCommandController({ client: facade, appendNotice: line => lines.push(line) })
  t.after(() => controller.stop())
  controller.start()
  await until(() => source.pending.length === 1)
  const pivot = beginMutableLocalIpcClientPivot(facade, target.client)
  await until(() => target.pending.length === 1)
  assert.equal(target.lists, 1)
  assert.equal(target.polls[0]!.after, 1)
  source.use(); source.pending.shift()!()
  await delay(30)
  assert.equal(lines.length, 0, "old source response must not publish a notice")
  assert.equal(source.polls.length, 1, "retained source must not continue polling")
  target.use(); target.pending.shift()!()
  await until(() => lines.length === 1 && target.pending.length === 1)
  assert.match(lines[0]!, /Agent target used retained access/)
  assert.equal(target.polls[1]!.after, 2)
  await delay(30)
  assert.equal(target.polls.length, 2, "lower destination cursor must not cause a request loop")
  await pivot.rollback()
  await until(() => source.pending.length === 1)
  assert.equal(source.lists, 2, "rollback must start a fresh source snapshot")
  assert.equal(source.polls.at(-1)!.after, 101)
  assert.match(lines.at(-1)!, /Agent source used retained access/)
  controller.stop()
  source.use(); source.pending.shift()!()
  await delay(30)
  assert.equal(lines.length, 2, "cleanup must fence late replies")
})

test("same-client real-IPC replay after kernel restart resets the cursor and retained-use projection", async t => {
  const kernel = await peer(t, 100, "holder")
  kernel.hold()
  const facade = createMutableLocalIpcClient(kernel.client), lines: string[] = []
  const controller = createAccessCommandController({ client: facade, appendNotice: line => lines.push(line) })
  t.after(() => controller.stop())
  controller.start()
  await until(() => kernel.pending.length === 1)
  kernel.use(); kernel.pending.shift()!()
  await until(() => lines.length === 1 && kernel.pending.length === 1)
  const interrupted = kernel.requests.at(-1)!
  assert.equal(interrupted.request.KernelBrowser.command.after, 101)
  kernel.restart()
  await until(() => kernel.connections === 2 && kernel.lists === 2 && kernel.pending.length === 1)
  assert.equal(facade.currentClient(), kernel.client, "the selected client was never swapped")
  const replay = kernel.requests[3]!
  assert.deepEqual(replay.request, interrupted.request, "IPC successfully retries the interrupted observation")
  // Browser observations use new receipts for the replacement terminal caller.
  assert.notEqual(replay.request_id, interrupted.request_id)
  assert.notEqual(replay.command_id, interrupted.command_id)
  assert.deepEqual(kernel.polls.map(poll => poll.after), [100, 101, 101, 1])
  await delay(30)
  assert.equal(kernel.polls.length, 4, "the restarted kernel's lower cursor must not cause a request loop")
  kernel.use(); kernel.pending.shift()!()
  await until(() => lines.length === 2 && kernel.pending.length === 1)
  assert.equal(kernel.polls.at(-1)!.after, 2)
  assert.equal(lines[1], lines[0], "restart clears notice deduplication even for identical retained-use notices")
})

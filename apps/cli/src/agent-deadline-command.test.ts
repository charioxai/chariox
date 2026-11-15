import assert from "node:assert/strict"
import test from "node:test"
import { runAgentDeadline } from "./agent-deadline-command.js"

test("deadline resolves an agent and sends an absolute time or explicit clear", async () => {
  const calls: Record<string, unknown>[] = []
  const send = async (body: Record<string, unknown>) => {
    calls.push(body)
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "room" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: "agent", agent_ref: "a1", alias: "worker", session_id: "room" }] } }
    return { AgentDeadline: { deadline: null } }
  }
  await runAgentDeadline("worker", "30m", undefined, send, 1000)
  assert.deepEqual(calls.at(-1), { SetAgentDeadline: { session_id: "room", agent_id: "agent", deadline_ms: 1801000 } })
  await runAgentDeadline("a1", "none", "room", send, 1000)
  assert.deepEqual(calls.at(-1), { SetAgentDeadline: { session_id: "room", agent_id: "agent", deadline_ms: null } })
  for (const value of ["0m", "later", "-1h", "999999999999999999d"]) {
    await assert.rejects(runAgentDeadline("worker", value, undefined, send, 1000), /duration/)
  }
})

test("deadline refuses an ambiguous alias before changing anything", async () => {
  const send = async (body: Record<string, unknown>) => {
    if ("ListSessions" in body) return { SessionsListed: { sessions: [{ id: "one" }, { id: "two" }] } }
    if ("ListAgents" in body) return { AgentsListed: { agents: [{ id: JSON.stringify(body), alias: "worker" }] } }
    assert.fail("ambiguous agent changed")
  }
  await assert.rejects(runAgentDeadline("worker", "none", undefined, send), /multiple agents/)
})

test("the owner CLI uses the ordinary authenticated endpoint without an external grant", async () => {
  const { WebSocketServer } = await import("ws")
  const { runAgentDeadlineCommand } = await import("./agent-deadline-command.js")
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve, reject) => { server.once("listening", resolve); server.once("error", reject) })
  const address = server.address() as { port: number }
  const calls: Record<string, unknown>[] = []
  server.on("connection", socket => socket.on("message", bytes => {
    const frame = JSON.parse(String(bytes))
    if (frame.type !== "request") return
    calls.push(frame.request)
    const response = "ListAgents" in frame.request
      ? { AgentsListed: { agents: [{ id: "agent", alias: "worker" }] } }
      : { AgentDeadline: { deadline: null } }
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response, error: null }))
  }))
  const before = { url: process.env.CHARIOX_KERNEL_URL, socket: process.env.CHARIOX_DAEMON_SOCKET, exit: process.exitCode, log: console.log }
  try {
    process.env.CHARIOX_KERNEL_URL = `ws://127.0.0.1:${address.port}`
    process.env.CHARIOX_DAEMON_SOCKET = `/var/tmp/deadline-owner-no-grant-${process.pid}.sock`
    console.log = () => {}
    assert.equal(await runAgentDeadlineCommand(["agent", "deadline", "worker", "none", "--session", "room"]), true)
    assert.deepEqual(calls.at(-1), { SetAgentDeadline: { session_id: "room", agent_id: "agent", deadline_ms: null } })
    assert.equal(process.exitCode, before.exit)
  } finally {
    if (before.url === undefined) delete process.env.CHARIOX_KERNEL_URL; else process.env.CHARIOX_KERNEL_URL = before.url
    if (before.socket === undefined) delete process.env.CHARIOX_DAEMON_SOCKET; else process.env.CHARIOX_DAEMON_SOCKET = before.socket
    process.exitCode = before.exit
    console.log = before.log
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

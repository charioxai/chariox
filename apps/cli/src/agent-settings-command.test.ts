import assert from "node:assert/strict"
import type { AddressInfo } from "node:net"
import test from "node:test"
import { WebSocketServer } from "ws"
import { runAgentSettingsCommand } from "./agent-settings-command.js"

test("owner CLI reads a hidden agent and requests a turn-end update on the ordinary endpoint", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve, reject) => {
    server.once("listening", resolve)
    server.once("error", reject)
  })
  const previous = { url: process.env.CHARIOX_KERNEL_URL, log: console.log, exit: process.exitCode }
  const agent = {
    id: "agent", session_id: "room", alias: "worker", agent_ref: "A2",
    provider: "codex", model: "current", effort: null, account_profile: "work",
  }
  const pending = { provider: "codex", model: "next", effort: null, account_profile: "work" }
  let accepted = false
  const requests: Record<string, unknown>[] = []
  const output: unknown[] = []
  server.on("connection", socket => socket.on("message", body => {
    const frame = JSON.parse(String(body))
    if (frame.type !== "request") return
    requests.push(frame.request)
    if ("UpdateAgentProfileAtTurnEnd" in frame.request) accepted = true
    const current = { ...agent, ...(accepted ? { pending_profile: pending } : {}) }
    const response = "GetAgentSettings" in frame.request
      ? { AgentSettings: { agent: current } }
      : { AgentProfileUpdated: { agent: current, applies: "at_turn_end" } }
    socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response, error: null }))
  }))
  try {
    process.env.CHARIOX_KERNEL_URL = `ws://127.0.0.1:${(server.address() as AddressInfo).port}`
    console.log = value => output.push(JSON.parse(String(value)))
    assert(await runAgentSettingsCommand(["agent", "settings", "set", "worker", "--model", "next", "--at-turn-end"]))
    assert(await runAgentSettingsCommand(["agent", "settings", "worker", "--session", "room"]))
    assert.deepEqual(requests, [
      { GetAgentSettings: { agent: "worker" } },
      { UpdateAgentProfileAtTurnEnd: { agent_id: "agent", session_id: "room", model: "next", clear_effort: false } },
      { GetAgentSettings: { session_id: "room", agent: "worker" } },
    ])
    assert.equal((output[0] as { status: string }).status, "pending until this turn ends")
    assert.deepEqual((output[1] as { pending: unknown }).pending, pending)
    assert.equal(process.exitCode, previous.exit)
  } finally {
    console.log = previous.log
    process.exitCode = previous.exit
    if (previous.url === undefined) delete process.env.CHARIOX_KERNEL_URL
    else process.env.CHARIOX_KERNEL_URL = previous.url
    await new Promise<void>(resolve => server.close(() => resolve()))
  }
})

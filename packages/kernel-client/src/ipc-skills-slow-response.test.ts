import assert from "node:assert/strict"
import test from "node:test"

import WebSocket, { WebSocketServer } from "ws"

import { LocalIpcClient } from "./ipc.js"

// MP-08/MP-10: a repository update fetches from the network, so its answer may take
// longer than the control stall window. The admitted socket cannot replay it.
test("a Skills request still being fetched is answered, not cut off at the control stall window", async (t) => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>(resolve => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const skillsRequests: string[] = []
  server.on("connection", socket => {
    socket.on("message", payload => {
      const frame = JSON.parse(String(payload)) as { request_id: string; request?: Record<string, unknown> }
      const reply = (response: unknown, delayMs: number) => setTimeout(() => {
        if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "response", request_id: frame.request_id, response, error: null }))
      }, delayMs)
      if (JSON.stringify(frame).includes("RelayStatus")) {
        reply({ RelayStatus: { status: { daemon_id: "home", machine_id: "machine", capabilities: ["skills_v2_runtime", "skills_repository_update_v581"] } } }, 0)
        return
      }
      skillsRequests.push(frame.request_id)
      reply({ SkillsV2: { result: { kind: "loads", records: [] } } }, 700)
    })
  })
  const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
    kernelPingIntervalMs: 250, kernelMaxMissedPongs: 4,
    controlRequestRetryDeadlineMs: 3_000, controlResponseStallMs: 300, reconnectJitterMs: 0,
  })
  t.after(() => {
    client.destroy()
    for (const socket of server.clients) socket.terminate()
    return new Promise<void>(resolve => server.close(() => resolve()))
  })
  const answer = await client.send({ SkillsV2: { action: "resolve_repository_update", package_id: "package-1" } })
  assert.deepEqual(answer, { SkillsV2: { result: { kind: "loads", records: [] } } })
  assert.equal(skillsRequests.length, 1)
})

// MP-08 / MP-10 / MP-11: exercise real proxy WebSockets; not SV acceptance.
import assert from "node:assert/strict"
import { once } from "node:events"
import test from "node:test"
import WebSocket, { WebSocketServer } from "ws"
import type { LocalIpcClient } from "../ipc.js"
import { startCodexProxy } from "./codex-proxy.js"

test("native skill completion projects home catalog, leaves kernel inventory raw, and hot reloads", { timeout: 5_000 }, async (t) => {
  const upstream = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await once(upstream, "listening")
  const address = upstream.address()
  assert.ok(address && typeof address !== "string")
  upstream.on("connection", socket => socket.on("message", raw => {
    const request = JSON.parse(raw.toString())
    socket.send(JSON.stringify({ id: request.id, result: request.method === "skills/list"
      ? { data: [{ cwd: "/actual", skills: [{ name: "provider-only", description: "Native", path: "/native/SKILL.md", enabled: true }], errors: [] }] }
      : {} }))
  }))
  let revision = 1, catalogRequests = 0
  const proxy = await startCodexProxy({ upstreamEndpoint: `ws://127.0.0.1:${address.port}`,
    client: { send: async (request: any) => {
      if (request.GetSessionState) return { SessionState: { session: { project_id: "project" } } }
      assert.equal(request.SkillsV2.action, "catalog")
      assert.equal(request.SkillsV2.agent_id, "agent")
      catalogRequests++
      return { SkillsV2: { result: { kind: "catalog", revision, packages: [], native: [], metadata_budget_percent: 1,
        skills: [{ skill: { id: "home-skill", name: `chariox-${revision}`, description: "Current home instruction", short_description: null },
          effective_state: "manual", delivery: { kind: "chariox" }, project_override: null, also_in_provider: null }] } } }
    } } as unknown as LocalIpcClient,
    sessionId: "session", attachmentId: "attachment", agentId: "agent", model: "fixture", effort: "low",
    bindState: { promise: null, run: null }, inlineLocalAttachments: false, debug: () => {},
  })
  const proxyAddress = proxy.address()
  assert.ok(proxyAddress && typeof proxyAddress !== "string")
  const native = new WebSocket(`ws://127.0.0.1:${proxyAddress.port}`)
  const kernel = new WebSocket(`ws://127.0.0.1:${proxyAddress.port}`)
  t.after(async () => {
    native.close(); kernel.close()
    await new Promise<void>(resolve => proxy.close(() => resolve()))
    await new Promise<void>(resolve => upstream.close(() => resolve()))
  })
  await Promise.all([once(native, "open"), once(kernel, "open")])
  const request = (socket: WebSocket, id: number, method: string, params: any = {}) => new Promise<any>(resolve => {
    const listener = (raw: WebSocket.RawData) => {
      const response = JSON.parse(raw.toString())
      if (response.id === id) { socket.off("message", listener); resolve(response) }
    }
    socket.on("message", listener); socket.send(JSON.stringify({ id, method, params }))
  })
  await request(native, 1, "initialize", { clientInfo: { name: "codex_cli_rs", version: "fixture" } })
  await request(kernel, 1, "initialize", { clientInfo: { name: "chariox-kernel", version: "fixture" } })
  const completion = await request(native, 2, "skills/list", { cwds: ["/actual"], forceReload: true })
  assert.deepEqual(completion.result.data[0].skills.map((s: any) => s.name), ["provider-only", "chariox-1"])
  assert.equal(completion.result.data[0].skills[1].path, "chariox-skill://home-skill/SKILL.md")
  const beforeKernelQuery = catalogRequests
  const inventory = await request(kernel, 2, "skills/list", { cwds: ["/actual"], forceReload: true })
  assert.deepEqual(inventory.result.data[0].skills.map((s: any) => s.name), ["provider-only"])
  assert.equal(catalogRequests, beforeKernelQuery)
  const invalidated = new Promise<void>(resolve => {
    const listener = (raw: WebSocket.RawData) => {
      if (JSON.parse(raw.toString()).method === "skills/changed") { native.off("message", listener); resolve() }
    }
    native.on("message", listener)
  })
  revision = 2
  await proxy.refreshSkills(); await invalidated
  const refreshed = await request(native, 3, "skills/list", { cwds: ["/actual"], forceReload: true })
  assert.deepEqual(refreshed.result.data[0].skills.map((s: any) => s.name), ["provider-only", "chariox-2"])
})

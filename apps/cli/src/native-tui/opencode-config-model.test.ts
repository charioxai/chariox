import assert from "node:assert/strict"
import http from "node:http"
import { once } from "node:events"
import test from "node:test"
import { startOpenCodeProxy } from "./opencode-proxy.js"
import type { LocalIpcClient } from "../ipc.js"
import type { RuntimeProviderRun } from "../cli-types.js"

test("MP-08 MP-10 MP-11 native OpenCode starts with the admitted model and preserves later user changes", async () => {
  const upstream = http.createServer((_request, response) => { response.setHeader("content-type", "application/json"); response.end(JSON.stringify({ model: "opencode/big-pickle", theme: "system" })) })
  upstream.listen(0, "127.0.0.1"); await once(upstream, "listening")
  const address = upstream.address(); assert(address && typeof address !== "string")
  let run = { id: "run", session_id: "session", agent_instance_id: "agent", provider: "opencode", state: "Running", model: "openai/gpt-6.1-sol" } as RuntimeProviderRun
  const requests: any[] = []
  const proxy = await startOpenCodeProxy({ upstreamBaseUrl: `http://127.0.0.1:${address.port}`, sessionId: "session", attachmentId: "attachment", agentId: "agent", inlineLocalAttachments: false, debug: () => {}, client: { send: async (request: any) => {
    requests.push(request)
    if (request.GetProviderRun) return { ProviderRun: { provider_run: run } }
    if (request.UpdateProviderRunSelection) { run = { ...run, model: request.UpdateProviderRunSelection.model }; return { ProviderRunSelectionUpdated: { provider_run: run } } }
    return { PromptSubmitted: {} }
  } } as unknown as LocalIpcClient })
  proxy.bindProviderRun(run)
  const port = proxy.server.address(); assert(port && typeof port !== "string")
  const base = `http://127.0.0.1:${port.port}`
  try {
    assert.deepEqual(await (await fetch(base + "/config?directory=public-workspace")).json(), { model: "openai/gpt-6.1-sol", theme: "system" })
    assert.equal((await (await fetch(base + "/config", { headers: { "x-chariox-provider-client": "kernel" } })).json() as any).model, "opencode/big-pickle")
    await fetch(base + "/session/provider-session/prompt_async", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: { providerID: "openai", modelID: "gpt-6.1-sol" }, parts: [{ type: "text", text: "read public docs" }] }) })
    assert.equal(requests.filter(r => r.UpdateProviderRunSelection).length, 0)
    await fetch(base + "/session/provider-session/prompt_async", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ model: { providerID: "openai", modelID: "user-selected-model" }, parts: [{ type: "text", text: "continue public docs" }] }) })
    assert.equal(requests.filter(r => r.UpdateProviderRunSelection).length, 1, "later native model choice remains authoritative")
  } finally { await new Promise<void>(resolve => proxy.server.close(() => resolve())); await new Promise<void>(resolve => upstream.close(() => resolve())) }
})

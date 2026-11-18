import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "../ipc.js"
import { startOpenCodeProxy } from "./opencode-proxy.js"

// A prompt the kernel refuses reaches the person as the kernel's reason, not as
// a bare "502 Bad Gateway" (OpenCode's toast prints the status line).
test("a refused OpenCode prompt names the reason in the status line", async (t) => {
  const reason = "OpenCode model `x` needs an OpenCode account that is not connected. Add the model's provider in Provider Accounts."
  const client = {
    send: async (request: Record<string, unknown>) => {
      if ("GetProviderRun" in request) return { ProviderRun: { provider_run: { id: "run-1", model: "opencode/x", variant: null } } }
      if ("SubmitPrompt" in request) throw new Error(reason)
      throw new Error(`unexpected request ${Object.keys(request)}`)
    },
  } as unknown as LocalIpcClient
  const proxy = await startOpenCodeProxy({
    upstreamBaseUrl: "http://127.0.0.1:1", client,
    sessionId: "home-session", attachmentId: "attachment-1", agentId: "agent-1",
    inlineLocalAttachments: true, debug: () => {},
  })
  proxy.bindProviderRun({
    id: "run-1", session_id: "home-session", agent_instance_id: "agent-1",
    adapter_key: "opencode", provider: "opencode", account_profile: "test",
    model: "opencode/x", variant: null, usage_tokens_total: null,
    state: "Running", provider_session_id: "provider-session",
  })
  t.after(() => new Promise<void>((resolve) => proxy.server.close(() => resolve())))
  const address = proxy.server.address()
  assert(address && typeof address !== "string")
  const response = await fetch(`http://127.0.0.1:${address.port}/session/provider-session/message`, {
    method: "POST", headers: { "content-type": "application/json" },
    body: JSON.stringify({ parts: [{ type: "text", text: "hello" }] }),
  })
  await response.arrayBuffer()
  assert.equal(response.status, 502)
  assert.match(response.statusText, /needs an OpenCode account.*Provider Accounts/)
})

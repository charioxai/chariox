import assert from "node:assert/strict"
import http from "node:http"
import { once } from "node:events"
import test from "node:test"

import type { LocalIpcClient } from "../ipc.js"
import { openCodeNativePermissionChoice } from "./opencode-permission.js"
import { startOpenCodeProxy } from "./opencode-proxy.js"

// MP-08/MP-10/MP-11: current official permission replies must resolve the
// exact kernel interaction rather than approving only the upstream provider.
for (const [reply, choice] of [["once", "allow_once"], ["always", "allow_session"], ["reject", "deny"]]) {
  test(`MP-11 current OpenCode reply ${reply} preserves ${choice}`, () => {
    assert.equal(openCodeNativePermissionChoice({ reply }), choice)
  })
}

test("MP-11 unknown OpenCode permission choices fail closed", () => {
  assert.equal(openCodeNativePermissionChoice({ reply: "not-a-decision" }), null)
  assert.equal(openCodeNativePermissionChoice({}), null)
})

for (const [route, body, expectedStatus, choice] of [
  ["/permission/per-current/reply", { reply: "once" }, 204, "allow_once"],
  ["/permission/per-current/reply", { reply: "always" }, 204, "allow_session"],
  ["/permission/per-current/reply", { reply: "reject" }, 204, "deny"],
  ["/permission/per-old/reply", { reply: "once" }, 409, null],
  ["/permission/per-current/reply", { reply: "unknown" }, 400, null],
  ["/session/provider-session/permissions/per-current", { response: "reject" }, 204, "deny"],
  ["/session/provider-session/permissions/per-old", { response: "once" }, 409, null],
  ["/session/another-session/permissions/per-current", { response: "once" }, 409, null],
  ["/api/session/provider-session/permission/per-current/reply", { reply: "reject" }, 204, "deny"],
  ["/api/session/provider-session/permission/per-old/reply", { reply: "once" }, 409, null],
  ["/api/session/another-session/permission/per-current/reply", { reply: "once" }, 409, null],
] as const) {
  test(`MP-08/MP-10/MP-11 ${route} ${JSON.stringify(body)} resolves only its matching interaction`, async (t) => {
    let upstreamReplies = 0
    const upstream = http.createServer((_request, response) => {
      upstreamReplies += 1
      response.writeHead(200, { "content-type": "application/json" })
      response.end("true")
    })
    upstream.listen(0, "127.0.0.1")
    await once(upstream, "listening")
    const address = upstream.address()
    assert(address && typeof address !== "string")
    t.after(() => new Promise<void>((resolve) => upstream.close(() => resolve())))
    const responses: Record<string, unknown>[] = []
    const client = {
      async send(request: Record<string, unknown>) {
        if ("GetSessionState" in request) return { SessionState: { session: {
          id: "home-session", agents: [], active_provider_run_id: "run-1",
          active_prompt: null, queued_prompts: [],
          active_interactions: [{
            id: "opencode-permission-per-current", agent_id: "agent-1", kind: "permission",
            level: "warning", message: "Allow research tool?", requested_at_ms: 1,
            choices: [{ id: "allow_once", label: "Allow once", reply: "allow_once" }],
          }],
        } } }
        if ("RespondToInteraction" in request) {
          responses.push(request)
          return { InteractionResponded: { session: {} } }
        }
        throw new Error(`unexpected request ${Object.keys(request)}`)
      },
    } as unknown as LocalIpcClient
    const proxy = await startOpenCodeProxy({
      upstreamBaseUrl: `http://127.0.0.1:${address.port}`, client,
      sessionId: "home-session", attachmentId: "attachment-1", agentId: "agent-1",
      inlineLocalAttachments: true, debug: () => {},
    })
    proxy.bindProviderRun({
      id: "run-1", session_id: "home-session", agent_instance_id: "agent-1",
      adapter_key: "opencode", provider: "opencode", account_profile: "test",
      model: "opencode/big-pickle", variant: null, usage_tokens_total: null,
      state: "Running", provider_session_id: "provider-session",
    })
    t.after(() => new Promise<void>((resolve) => proxy.server.close(() => resolve())))
    const proxyAddress = proxy.server.address()
    assert(proxyAddress && typeof proxyAddress !== "string")
    const response = await fetch(`http://127.0.0.1:${proxyAddress.port}${route}`, {
      method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body),
    })
    await response.arrayBuffer()
    assert.equal(response.status, expectedStatus)
    assert.equal(upstreamReplies, 0, "native replies must pass through kernel admission")
    assert.deepEqual(responses, choice === null ? [] : [{ RespondToInteraction: {
      session_id: "home-session", interaction_id: "opencode-permission-per-current",
      choice_id: choice, custom_reply: null,
    } }])
  })
}

import assert from "node:assert/strict"
import { request } from "node:http"
import test from "node:test"

import { startClaudePermissionBridge, type ClaudePromptOriginState } from "./claude-permission-bridge.js"

const permission = { hook_event_name: "PermissionRequest", permission_mode: "default", tool_name: "Bash" }

test("a delayed Claude approval forwards its captured origin after a follow-up starts", async () => {
  const current: ClaudePromptOriginState = { current: "native" }
  const sent: unknown[] = []
  const bridge = await startClaudePermissionBridge({
    client: { send: async (value: unknown) => {
      sent.push(value)
      return { NativeProviderInteractionResolved: { resolution: { status: "timed_out" } } }
    } } as never,
    sessionId: "session", agentId: "agent", attachmentId: "attachment", promptOrigin: current,
  })
  try {
    const body = JSON.stringify({ ...permission, native_origin: {
      scope: "prompt", provider_run_id: "run-A", prompt_id: "prompt-A",
    } })
    const incoming = request(new URL("/permission", bridge.url), { method: "POST" })
    const result = new Promise<Record<string, unknown>>((resolve, reject) => {
      incoming.on("response", (response) => {
        let text = ""
        response.setEncoding("utf8")
        response.on("data", (chunk) => { text += chunk })
        response.on("end", () => resolve(JSON.parse(text)))
      })
      incoming.on("error", reject)
    })
    incoming.write(body.slice(0, 20))
    current.current = "external"
    incoming.end(body.slice(20))
    assert.equal((await result).behavior, "deny")
    assert.deepEqual((sent[0] as { RequestNativeProviderTurnInteraction: { origin: unknown } })
      .RequestNativeProviderTurnInteraction.origin,
    { scope: "prompt", provider_run_id: "run-A", prompt_id: "prompt-A" })
  } finally {
    await bridge.stop()
  }
})

test("a Claude approval without a captured turn fails closed before kernel dispatch", async () => {
  let dispatched = false
  const bridge = await startClaudePermissionBridge({
    client: { send: async () => { dispatched = true; throw new Error("unexpected dispatch") } } as never,
    sessionId: "session", agentId: "agent", attachmentId: "attachment", promptOrigin: { current: "native" },
  })
  try {
    const response = await fetch(new URL("/permission", bridge.url), {
      method: "POST", body: JSON.stringify(permission),
    })
    const decision = await response.json() as { behavior: string }
    assert.equal(decision.behavior, "deny")
    assert.equal(dispatched, false)
  } finally {
    await bridge.stop()
  }
})

// MP-08 / MP-10 / MP-11: native provider setup grants tools, never skill instructions.
import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { grantNativeCapabilities } from "./capability-grants.js"
test("native setup cannot turn a skill instruction into an extension grant", async () => {
  const requests: any[] = []
  const client = { send: async (request: any) => {
    requests.push(request)
    return { AgentExtensionGranted: { agent: { extension_grants: [{ kind: "mcp", name: "tools" }, { kind: "skill", name: "review" }] } } }
  } } as unknown as LocalIpcClient
  await (grantNativeCapabilities as (...args: any[]) => Promise<void>)(client, "/work", "agent", ["tools"], ["review"])
  assert.equal(requests.length, 1)
  assert.equal(requests.some(r => r.GrantAgentExtension?.kind === "skill"), false)
})

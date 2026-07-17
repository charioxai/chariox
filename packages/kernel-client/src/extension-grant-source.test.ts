import assert from "node:assert/strict"
import test from "node:test"

import {
  extensionGrantKey,
  extensionGrantSource,
  extensionIdentityKey,
} from "./extension-grant-source.js"
import {
  grantAgentExtensionRequest,
  listAgentExtensionCatalogRequest,
  revokeAgentExtensionRequest,
} from "./ipc-extension-requests.js"

test("legacy extension grants default to the home source", () => {
  assert.equal(extensionGrantSource({}), "home")
  assert.equal(extensionGrantSource({ source: "home" }), "home")
  assert.equal(extensionGrantSource({ source: "worker" }), "worker")
})

test("extension grant identity includes source", () => {
  assert.equal(extensionGrantKey({ kind: "mcp", name: "browser" }), "home:mcp:browser")
  assert.equal(extensionGrantKey({ source: "worker", kind: "mcp", name: "browser" }), "worker:mcp:browser")
  assert.equal(extensionIdentityKey("worker", "script", "lookup"), "worker:script:lookup")
})

test("extension source request builders match the source-aware kernel wire shape", () => {
  assert.deepEqual(
    grantAgentExtensionRequest("/repo", "agent-1", "script", "lookup", "python", { source: "worker" }),
    {
      GrantAgentExtension: {
        workspace_id: "/repo",
        agent_ref: "agent-1",
        source: "worker",
        kind: "script",
        name: "lookup",
        environment: "python",
      },
    },
  )
  assert.deepEqual(revokeAgentExtensionRequest("agent-1", "script", "lookup", "worker"), {
    RevokeAgentExtension: {
      agent_ref: "agent-1",
      source: "worker",
      kind: "script",
      name: "lookup",
    },
  })
  assert.deepEqual(listAgentExtensionCatalogRequest("agent-1"), {
    ListAgentExtensionCatalog: { agent_ref: "agent-1", source: "all" },
  })
  assert.deepEqual(listAgentExtensionCatalogRequest("agent-1", "worker"), {
    ListAgentExtensionCatalog: { agent_ref: "agent-1", source: "worker" },
  })
})

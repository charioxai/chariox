import assert from "node:assert/strict"
import test from "node:test"

import {
  formatSliceProviderAccounts,
  formatSliceScope,
} from "./slice-format.js"

test("slice formatter projects provider account identities", () => {
  assert.equal(formatSliceProviderAccounts({ provider_auth: [] }), "none")
  assert.equal(
    formatSliceProviderAccounts({
      provider_auth: [{
        provider: "codex",
        state: "authenticated",
        email: "dev@example.com",
        alias: "daily",
      }],
    }),
    "codex=daily (dev@example.com)",
  )
})

test("slice formatter preserves provider auth attention states", () => {
  assert.equal(
    formatSliceProviderAccounts({
      provider_auth: [{
        provider: "opencode",
        state: "not_configured",
      }],
    }),
    "opencode=not_configured/state=not_configured",
  )
})

test("slice formatter resolves worktree scope in display order", () => {
  assert.equal(formatSliceScope({ worktree_id: "/repo/worktree", workspace_mount: "/workspace", workspace_id: "workspace-1" }), "/repo/worktree")
  assert.equal(formatSliceScope({ workspace_mount: "/workspace", workspace_id: "workspace-1" }), "/workspace")
  assert.equal(formatSliceScope({ workspace_id: "workspace-1" }), "workspace-1")
  assert.equal(formatSliceScope({}), "-")
})

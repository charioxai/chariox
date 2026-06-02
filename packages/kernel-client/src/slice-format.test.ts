import assert from "node:assert/strict"
import test from "node:test"

import {
  formatSliceProviderAccounts,
  formatSliceProviderAuthStatus,
  formatSliceScope,
  sliceProviderAuthCoverage,
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

test("slice formatter reports advertised provider auth coverage", () => {
  assert.deepEqual(
    sliceProviderAuthCoverage({
      providers: ["codex", "opencode"],
      provider_auth: [{ provider: "opencode:openai", state: "configured" }],
    }),
    {
      providers: ["codex", "opencode"],
      authProviders: ["opencode:openai"],
      missingProviders: ["codex"],
      staleProviders: [],
      hasHealthyCoverage: false,
      needsAttention: true,
    },
  )
  assert.deepEqual(
    sliceProviderAuthCoverage({
      providers: ["opencode"],
      provider_auth: [
        { provider: "opencode:openai", state: "configured" },
        { provider: "opencode:opencode", state: "unknown" },
      ],
    }),
    {
      providers: ["opencode"],
      authProviders: ["opencode:openai", "opencode:opencode"],
      missingProviders: [],
      staleProviders: ["opencode:opencode"],
      hasHealthyCoverage: false,
      needsAttention: true,
    },
  )
})

test("slice formatter summarizes partial provider auth status", () => {
  assert.equal(
    formatSliceProviderAuthStatus({
      providers: ["codex", "opencode"],
      provider_auth: [{ provider: "codex", state: "configured", alias: "daily" }],
    }),
    "auth codex=daily (configured); missing opencode",
  )
  assert.equal(
    formatSliceProviderAuthStatus({
      providers: ["opencode"],
      provider_auth: [
        { provider: "opencode:openai", state: "configured" },
        { provider: "opencode:opencode", state: "not_configured" },
      ],
    }),
    "auth opencode:openai=configured, opencode:opencode=not_configured/state=not_configured; refresh opencode:opencode",
  )
})

test("slice formatter resolves worktree scope in display order", () => {
  assert.equal(formatSliceScope({ worktree_id: "/repo/worktree", workspace_mount: "/workspace", workspace_id: "workspace-1" }), "/repo/worktree")
  assert.equal(formatSliceScope({ workspace_mount: "/workspace", workspace_id: "workspace-1" }), "/workspace")
  assert.equal(formatSliceScope({ workspace_id: "workspace-1" }), "workspace-1")
  assert.equal(formatSliceScope({}), "-")
})

import assert from "node:assert/strict"
import test from "node:test"

import {
  formatProviderAccountForBackend,
  formatSliceBackendProviderAccount,
  formatSliceProviderAccounts,
  formatSliceProviderAuthReadiness,
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
        account_profile: "daily",
      }],
    }),
    "codex=daily (dev@example.com)",
  )
})

test("slice formatter resolves backend provider account labels", () => {
  const slice = {
    providers: ["codex", "opencode", "claude"],
    provider_auth: [
      { provider: "codex", state: "configured", alias: "daily", email: "dev@example.com" },
      { provider: "opencode:openai", state: "configured", account_id: "acct-1234567890abcdef" },
      { provider: "claude", state: "not_configured" },
    ],
  }

  assert.equal(formatSliceBackendProviderAccount(slice, "codex"), "daily")
  assert.equal(formatSliceBackendProviderAccount(slice, "opencode"), "acct-123...cdef")
  assert.equal(formatSliceBackendProviderAccount(slice, "claude"), "auth missing")
  assert.equal(formatSliceBackendProviderAccount({
    providers: ["codex"],
    provider_auth: [{ provider: "codex", state: "configured" }],
  }, "codex"), "account unknown")
  assert.equal(formatSliceBackendProviderAccount({ providers: ["codex"], provider_auth: [] }, "codex"), "auth missing")
  assert.equal(formatSliceBackendProviderAccount(slice, "unknown"), null)
})

test("provider account formatter resolves backend labels without a slice", () => {
  assert.equal(
    formatProviderAccountForBackend(
      [{ provider: "codex", account_profile: "worker-codex", state: "configured" }],
      "codex",
      ["codex"],
    ),
    "worker-codex",
  )
  assert.equal(formatProviderAccountForBackend([], "codex", ["codex"]), "auth missing")
  assert.equal(formatProviderAccountForBackend([], "claude", ["codex"]), null)
})

test("slice formatter preserves provider auth attention states", () => {
  assert.equal(
    formatSliceProviderAccounts({
      provider_auth: [{
        provider: "opencode",
        state: "not_configured",
      }],
    }),
    "opencode=auth missing/state=not_configured",
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
      provider_auth: [{ provider: "codex", account_profile: "daily", state: "configured" }],
    }),
    "auth codex=daily (account unknown); missing opencode",
  )
  assert.equal(
    formatSliceProviderAuthStatus({
      providers: ["opencode"],
      provider_auth: [
        { provider: "opencode:openai", state: "configured" },
        { provider: "opencode:opencode", state: "not_configured" },
      ],
    }),
    "auth opencode:openai=account unknown, opencode:opencode=auth missing/state=not_configured; refresh opencode:opencode",
  )
})

test("slice formatter summarizes provider auth readiness for scanning", () => {
  assert.equal(
    formatSliceProviderAuthReadiness({
      providers: ["codex", "claude"],
      provider_auth: [
        { provider: "codex", state: "configured" },
        { provider: "claude", state: "authenticated" },
      ],
    }),
    "ready codex, claude",
  )
  assert.equal(
    formatSliceProviderAuthReadiness({
      providers: ["codex", "opencode:openai"],
      provider_auth: [{ provider: "codex", state: "configured" }],
    }),
    "missing opencode:openai",
  )
  assert.equal(
    formatSliceProviderAuthReadiness({
      providers: ["opencode"],
      provider_auth: [
        { provider: "opencode:openai", state: "configured" },
        { provider: "opencode:opencode", state: "not_configured" },
      ],
    }),
    "refresh opencode:opencode",
  )
  assert.equal(formatSliceProviderAuthReadiness({ providers: [], provider_auth: [] }), "no provider targets")
})

test("slice formatter resolves worktree scope in display order", () => {
  assert.equal(formatSliceScope({ worktree_id: "/repo/worktree", workspace_mount: "/workspace", workspace_id: "workspace-1" }), "/repo/worktree")
  assert.equal(formatSliceScope({ workspace_mount: "/workspace", workspace_id: "workspace-1" }), "/workspace")
  assert.equal(formatSliceScope({ workspace_id: "workspace-1" }), "workspace-1")
  assert.equal(formatSliceScope({}), "-")
})

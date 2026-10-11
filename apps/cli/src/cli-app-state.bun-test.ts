import assert from "node:assert/strict"
import test from "node:test"
import { createCliAppState } from "./cli-app-state.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
import { selectedProviderAccountStartBlockReason } from "./waiting-room-provider-accounts.js"

test("MP-08 / MP-11 cold account inventory does not tell a signed-in agent to sign in", () => {
  const state = createCliAppState({
    cwd: "/workspace",
    bootstrap: {
      client: { supportsKernelEvents: () => false } as never,
      binding: null, sessions: [],
      providerCatalog: fallbackProviderCatalog(),
      providerCommandCatalogs: fallbackProviderCommandCatalogs(),
      terminalCommandCatalog: null,
      options: { clientId: "test", model: "default", accountProfile: "default", effort: "" },
      preferences: {},
    },
  })
  assert.equal(selectedProviderAccountStartBlockReason(state.providerAccountsState(), "codex", "signed-in"), null)
})

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

test("MP-08 / MP-10 a startup attach with deferred history starts hydrating, never as an empty session", () => {
  const bootstrap = (deferred: object | undefined) => createCliAppState({
    cwd: "/workspace",
    bootstrap: {
      client: { supportsKernelEvents: () => false } as never,
      binding: null, sessions: [],
      providerCatalog: fallbackProviderCatalog(),
      providerCommandCatalogs: fallbackProviderCommandCatalogs(),
      terminalCommandCatalog: null,
      options: { clientId: "test", model: "default", accountProfile: "default", effort: "" },
      preferences: {},
      deferred,
    } as never,
  })
  assert.equal(bootstrap({ attachedHistory: new Promise(() => {}) }).sessionHydrating(), true)
  assert.equal(bootstrap(undefined).sessionHydrating(), false)
})

import assert from "node:assert/strict"
import test from "node:test"

import {
  fallbackProviderCommandCatalogs,
  parseProviderNamespaceCommand,
  providerNamespace,
} from "./provider-command-catalog.js"

test("provider command catalogs default to shipped empty catalogs", () => {
  const catalog = fallbackProviderCommandCatalogs().opencode
  assert.equal(catalog.source, "shipped")
  assert.equal(catalog.commands.length, 0)
})

test("parseProviderNamespaceCommand rewrites focused provider namespaces", () => {
  assert.deepEqual(parseProviderNamespaceCommand("/opencode compact", "opencode"), {
    raw: "/opencode compact",
    provider: "opencode",
    forwardedCommand: "/compact",
  })
  assert.deepEqual(parseProviderNamespaceCommand("/codex compact", "opencode"), {
    raw: "/codex compact",
    provider: "codex",
    forwardedCommand: "/compact",
  })
  assert.equal(providerNamespace("codex"), "/codex")
})

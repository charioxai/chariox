import assert from "node:assert/strict"
import test from "node:test"

import {
  fallbackProviderCommandCatalogs,
  parseProviderNamespaceCommand,
  providerCommandCatalogIsLocalFallback,
  providerNamespace,
  providerNamespaceDescription,
} from "./provider-command-catalog.js"

test("provider command catalogs default to shipped empty catalogs", () => {
  const catalog = fallbackProviderCommandCatalogs().opencode
  assert.equal(catalog.source, "shipped")
  assert.equal(catalog.commands.length, 0)
  assert.equal(providerCommandCatalogIsLocalFallback(catalog), false)
})

test("provider command catalogs can mark local fallback source", () => {
  const catalog = fallbackProviderCommandCatalogs({
    catalogSource: "local_fallback",
    unavailableReason: "daemon down",
  }).codex

  assert.equal(providerCommandCatalogIsLocalFallback(catalog), true)
  assert.equal(catalog.unavailable_reason, "daemon down")
  assert.match(providerNamespaceDescription("codex", 0, { localFallback: true }), /local command list/)
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
  assert.equal(parseProviderNamespaceCommand("/claude compact", "claude-p"), null)
  assert.equal(providerNamespace("codex"), "/codex")
})

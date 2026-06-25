import assert from "node:assert/strict"
import test from "node:test"

import {
  backendProviderLabel,
  catalogModelOptions,
  fallbackProviderCatalog,
  normalizeBackendProviderId,
  providerCatalogIsLocalFallback,
  providerDisplayName,
  type ProviderCatalog,
} from "./provider-catalog.js"

test("providerDisplayName appends remote machine aliases", () => {
  assert.equal(
    providerDisplayName({
      id: "codex",
      name: "Codex",
      remote_machine_aliases: ["builder-west"],
      models: {},
    }),
    "Codex (builder-west)",
  )
})

test("catalogModelOptions uses remote machine qualified provider names", () => {
  const catalog: ProviderCatalog = {
    all: [
      {
        id: "codex",
        name: "Codex",
        remote_machine_aliases: ["builder-west"],
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: { high: {} },
          },
        },
      },
    ],
    default: {
      codex: "gpt-5.4",
    },
    connected: ["codex"],
  }

  const options = catalogModelOptions(catalog, "codex")
  assert.equal(options.length, 1)
  assert.equal(options[0]?.providerName, "Codex (builder-west)")
})

test("fallback catalog exposes Claude headless and Claude -p as isolated backends", () => {
  const catalog = fallbackProviderCatalog()

  assert.equal(backendProviderLabel("claude-headless"), "Claude headless")
  assert.equal(backendProviderLabel("claude-p"), "Claude -p")
  assert.equal(backendProviderLabel("pi"), "Pi")
  assert.equal(normalizeBackendProviderId("claude"), "claude-p")

  const claudeHeadlessOptions = catalogModelOptions(catalog, "claude-headless")
  assert.deepEqual(claudeHeadlessOptions.map((option) => option.providerId), ["claude-headless"])
  assert.deepEqual(claudeHeadlessOptions.map((option) => option.id), ["claude-headless/claude-sonnet-4-6"])

  const claudePrintOptions = catalogModelOptions(catalog, "claude-p")
  assert.deepEqual(claudePrintOptions.map((option) => option.providerId), ["claude-p"])
  assert.deepEqual(claudePrintOptions.map((option) => option.id), ["claude-p/claude-sonnet-4-6"])

  const opencodeOptions = catalogModelOptions(catalog, "opencode")
  assert.equal(opencodeOptions.some((option) => option.providerId.startsWith("claude")), false)

  const piOptions = catalogModelOptions(catalog, "pi")
  assert.deepEqual(piOptions.map((option) => option.providerId), ["pi", "pi"])
  assert.deepEqual(piOptions.map((option) => option.id), [
    "pi/openai-codex/gpt-5.4",
    "pi/openai/gpt-5.4",
  ])
})

test("fallback catalog can be marked as local fallback metadata", () => {
  const catalog = fallbackProviderCatalog({
    source: "local_fallback",
    unavailableReason: "provider catalog unavailable",
  })

  assert.equal(providerCatalogIsLocalFallback(catalog), true)
  assert.equal(catalog.source, "local_fallback")
  assert.equal(catalog.unavailable_reason, "provider catalog unavailable")
  assert.equal(providerCatalogIsLocalFallback(fallbackProviderCatalog()), false)
})

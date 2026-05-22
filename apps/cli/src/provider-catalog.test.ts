import assert from "node:assert/strict"
import test from "node:test"

import {
  backendProviderLabel,
  catalogModelOptions,
  fallbackProviderCatalog,
  normalizeBackendProviderId,
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

test("fallback catalog exposes Claude Code as an isolated backend", () => {
  const catalog = fallbackProviderCatalog()

  assert.equal(backendProviderLabel("claude"), "Claude Code")
  assert.equal(normalizeBackendProviderId("claude"), "claude")

  const claudeOptions = catalogModelOptions(catalog, "claude")
  assert.deepEqual(claudeOptions.map((option) => option.providerId), ["claude"])
  assert.deepEqual(claudeOptions.map((option) => option.id), ["claude/claude-sonnet-4-6"])

  const opencodeOptions = catalogModelOptions(catalog, "opencode")
  assert.equal(opencodeOptions.some((option) => option.providerId === "claude"), false)
})

import assert from "node:assert/strict"
import test from "node:test"

import { catalogModelOptions, providerDisplayName, type ProviderCatalog } from "./provider-catalog.js"

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

import assert from "node:assert/strict"
import test from "node:test"

import {
  backendProviderLabel,
  catalogModelOptions,
  fallbackProviderCatalog,
  normalizeBackendProviderId,
  providerCatalogIsLocalFallback,
  providerDisplayName,
  selectConfiguredModel,
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
  assert.equal(normalizeBackendProviderId("claude"), "claude-p")

  const claudeHeadlessOptions = catalogModelOptions(catalog, "claude-headless")
  assert.deepEqual(claudeHeadlessOptions.map((option) => option.providerId), ["claude-headless"])
  assert.deepEqual(claudeHeadlessOptions.map((option) => option.id), ["claude-headless/claude-sonnet-4-6"])

  const claudePrintOptions = catalogModelOptions(catalog, "claude-p")
  assert.deepEqual(claudePrintOptions.map((option) => option.providerId), ["claude-p"])
  assert.deepEqual(claudePrintOptions.map((option) => option.id), ["claude-p/claude-sonnet-4-6"])

  const opencodeOptions = catalogModelOptions(catalog, "opencode")
  assert.equal(opencodeOptions.some((option) => option.providerId.startsWith("claude")), false)
})

test("OpenCode model selection includes Go, Zen, and arbitrary upstream providers", () => {
  const catalog: ProviderCatalog = {
    all: [
      provider("opencode-go", "OpenCode Go", "deepseek-v4-pro"),
      provider("opencode", "OpenCode Zen", "gpt-5.2"),
      provider("openai", "OpenAI", "gpt-5.2"),
      provider("codex", "Codex", "gpt-5.6"),
    ],
    default: {},
    connected: ["opencode-go", "opencode", "openai", "codex"],
  }

  assert.deepEqual(
    catalogModelOptions(catalog, "opencode").map((option) => option.providerId).sort(),
    ["openai", "opencode", "opencode-go"],
  )
})

function provider(id: string, name: string, modelId: string) {
  return {
    id,
    name,
    remote_machine_aliases: [],
    models: {
      [modelId]: { id: modelId, name: modelId, status: "active", variants: {} },
    },
  }
}

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

// MP-08 / MP-11: local fallback labels must obey the native catalog contract too.
test("fallback picker uses native names without Chariox branding", () => {
  const catalog = fallbackProviderCatalog()
  assert.equal(catalogModelOptions(catalog, "codex")[0]?.label, "gpt-5.4")
  assert.equal(catalogModelOptions(catalog, "claude-p")[0]?.label, "Sonnet 4.6")
})

test("picker preserves native provider names, ids and model display names", () => {
  const catalog: ProviderCatalog = {
    all: [provider("codex", "Codex", "gpt-6-luna"),
      provider("claude-p", "Claude -p", "claude-haiku-5-5"),
      provider("opencode", "OpenCode Zen", "deepseek-v4-flash-free")],
    connected: ["codex", "claude-p", "opencode"], default: {},
  }
  catalog.all[1]!.models["claude-haiku-5-5"]!.name = "Haiku"
  catalog.all[2]!.models["deepseek-v4-flash-free"]!.name = "DeepSeek V4 Flash (free)"
  assert.equal(catalogModelOptions(catalog, "codex")[0]?.label, "gpt-6-luna")
  assert.equal(catalogModelOptions(catalog, "claude-p")[0]?.label, "Haiku")
  assert.equal(catalogModelOptions(catalog, "opencode")[0]?.label, "DeepSeek V4 Flash (free)")
  assert.equal(catalogModelOptions(catalog, "opencode")[0]?.id, "opencode/deepseek-v4-flash-free")
})

// MP-08 / MP-11 saved rolling Claude aliases keep their family after the native catalog lists resolved ids only.
test("saved Claude aliases select the newest native id of their family on both backends", () => {
  const claudeModels = (id: string) => {
    const entry = provider(id, id, "claude-opus-5-5")
    for (const model of ["claude-haiku-4-5", "claude-haiku-5-5", "claude-sonnet-5-5", "claude-fable-5-1"]) {
      entry.models[model] = { id: model, name: model, status: "active", variants: {} }
    }
    return entry
  }
  const catalog: ProviderCatalog = {
    all: [claudeModels("claude-p"), claudeModels("claude-headless")],
    connected: ["claude-p", "claude-headless"],
    default: { "claude-p": "claude-opus-5-5", "claude-headless": "claude-opus-5-5" },
  }
  for (const backend of ["claude-p", "claude-headless"] as const) {
    for (const saved of ["haiku", "claude/haiku", "claude-p/haiku", "claude-headless/haiku"]) {
      assert.equal(selectConfiguredModel(catalog, saved, backend)?.id, `${backend}/claude-haiku-5-5`, `${backend} ${saved}`)
    }
    assert.equal(selectConfiguredModel(catalog, "sonnet", backend)?.id, `${backend}/claude-sonnet-5-5`)
    assert.equal(selectConfiguredModel(catalog, "fable", backend)?.id, `${backend}/claude-fable-5-1`)
    assert.equal(selectConfiguredModel(catalog, "opus", backend)?.id, `${backend}/claude-opus-5-5`)
    assert.equal(selectConfiguredModel(catalog, "claude-haiku-4-5", backend)?.id, `${backend}/claude-haiku-4-5`)
  }
})

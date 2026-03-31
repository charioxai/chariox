import assert from "node:assert/strict"
import test from "node:test"

import { buildCommandCenterItems } from "./command-center.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"

test("buildCommandCenterItems shows root slash commands", () => {
  const items = buildCommandCenterItems("/", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.label === "/provider"), true)
  assert.equal(items.some((item) => item.label === "/model"), true)
  assert.equal(items.some((item) => item.label === "/variant"), true)
  assert.equal(items.some((item) => item.label === "/view"), true)
  assert.equal(items.some((item) => item.label === "/exit"), true)
})

test("buildCommandCenterItems filters model options", () => {
  const items = buildCommandCenterItems("/model gpt", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "model")
  assert.equal(items[0]?.value, "openai/gpt-5.4")
})

test("buildCommandCenterItems filters variant options", () => {
  const items = buildCommandCenterItems("/variant med", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "variant")
  assert.equal(items[0]?.value, "medium")
})

test("buildCommandCenterItems closes exact trailing-space commands", () => {
  const items = buildCommandCenterItems("/agent delete ", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.deepEqual(items, [])
})

test("buildCommandCenterItems shows the delete agent command", () => {
  const items = buildCommandCenterItems("/agent del", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "/agent delete")
})

test("buildCommandCenterItems keeps provider options open after space", () => {
  const items = buildCommandCenterItems("/provider ", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "provider")
  assert.equal(items[0]?.value, "opencode")
})

test("buildCommandCenterItems shows multi-agent view options", () => {
  const items = buildCommandCenterItems("/view spl", {
    providerCatalog: fallbackProviderCatalog(),
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "split")
  assert.equal(items[0]?.value, "/view split")
})

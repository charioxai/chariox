import assert from "node:assert/strict"
import test from "node:test"

import {
  buildModelItems,
  buildProviderItems,
  buildProviderNamespaceItems,
  buildVariantItems,
  buildViewItems,
  providerNamespaceRootItem,
} from "./command-center-dynamic-items.js"
import { COMMAND_TREE } from "./command-center-tree.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs, type ProviderCommandCatalogs } from "./provider-command-catalog.js"

test("command center dynamic items project provider namespaces and completions", () => {
  const catalogs = withCodexCommand()

  assert.equal(providerNamespaceRootItem("codex", catalogs).searchAliases?.includes("resume"), true)
  assert.deepEqual(
    buildProviderNamespaceItems("/codex ", "codex", catalogs).map((item) => item.value),
    ["/codex resume "],
  )
})

test("command center dynamic items project provider, model, variant, and view choices", () => {
  const providerNode = COMMAND_TREE.find((node) => node.id === "provider")!
  const context = {
    providerCatalog: fallbackProviderCatalog(),
    currentProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.equal(buildProviderItems("/provider cla", providerNode)[0]?.value, "claude")
  assert.equal(buildModelItems("/model gpt", context)[0]?.kind, "model")
  assert.equal(buildVariantItems("/variant med", context)[0]?.value, "medium")
  assert.equal(buildViewItems("/view spl")[0]?.value, "/view split")
})

function withCodexCommand(): ProviderCommandCatalogs {
  const catalogs = fallbackProviderCommandCatalogs()
  catalogs.codex = {
    ...catalogs.codex,
    commands: [{
      id: "resume",
      name: "resume",
      description: "Resume the focused provider turn",
      value: "resume ",
    }],
  }
  return catalogs
}

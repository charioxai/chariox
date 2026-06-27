import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

import {
  buildModelItems,
  buildProviderItems,
  buildProviderNamespaceItems,
  buildVariantItems,
  buildViewItems,
  providerNamespaceRootItem,
} from "./command-center-dynamic-items.js"
import type { CommandNode } from "./command-center-tree-projection.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs, type ProviderCommandCatalogs } from "./provider-command-catalog.js"

const commandTree = JSON.parse(readFileSync(
  new URL("../../kernel/src/runtime/terminal_command_catalog/catalog.json", import.meta.url),
  "utf8",
)) as CommandNode[]

test("command center dynamic items project provider namespaces and completions", () => {
  const catalogs = withCodexCommand()

  assert.equal(providerNamespaceRootItem("codex", catalogs).searchAliases?.includes("resume"), true)
  assert.deepEqual(
    buildProviderNamespaceItems("/codex ", "codex", catalogs).map((item) => item.value),
    ["/codex resume "],
  )
})

test("command center marks provider namespace command lists from local fallback", () => {
  const catalogs = fallbackProviderCommandCatalogs({ catalogSource: "local_fallback" })

  assert.match(providerNamespaceRootItem("codex", catalogs).description, /local command list/)
  assert.match(buildProviderNamespaceItems("/codex ", "codex", catalogs)[0]?.description ?? "", /local command list/)
})

test("command center dynamic items project provider, model, variant, and view choices", () => {
  const providerNode = commandTree.find((node) => node.id === "provider")!
  const context = {
    providerCatalog: fallbackProviderCatalog(),
    currentProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.deepEqual(
    buildProviderItems("/provider cla", providerNode, context)
      .filter((item) => item.kind === "provider")
      .map((item) => item.value)
      .sort(),
    ["claude-headless", "claude-p"],
  )
  assert.equal(buildProviderItems("/provider proc", providerNode, context).some((item) => item.value === "/provider processes "), true)
  const teardownItem = buildProviderItems("/provider teardown", providerNode, context).find((item) => item.value === "/provider processes teardown ")
  assert.equal(teardownItem?.description, "Tear down safe daemon-tracked provider processes for one provider")
  assert.equal(buildModelItems("/model gpt", context)[0]?.kind, "model")
  assert.equal(buildVariantItems("/variant med", context)[0]?.value, "medium")
  assert.equal(buildViewItems("/view spl")[0]?.value, "/view split")
})

test("command center marks provider and model choices from local fallback provider catalog", () => {
  const providerNode = commandTree.find((node) => node.id === "provider")!
  const context = {
    providerCatalog: fallbackProviderCatalog({ source: "local_fallback" }),
    currentProvider: "opencode" as const,
    currentModel: "opencode/gpt-5.4",
    currentVariant: "high",
  }

  assert.match(buildProviderItems("/provider codex", providerNode, context)[0]?.description ?? "", /local provider list/)
  assert.match(buildModelItems("/model gpt", context)[0]?.description ?? "", /local provider list/)
  assert.match(buildVariantItems("/variant high", context)[0]?.description ?? "", /local provider list/)
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

import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

import { buildScopedCommandCenterItems } from "./command-center-scoped-items.js"
import type { CommandNode } from "./command-center-tree-projection.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

const commandTree = JSON.parse(readFileSync(
  new URL("../../kernel/src/runtime/terminal_command_catalog/catalog.json", import.meta.url),
  "utf8",
)) as CommandNode[]

const context = {
  commandTree,
  focusedProvider: "opencode" as const,
  providerCommandCatalogs: fallbackProviderCommandCatalogs(),
}

test("command center scoped items keep the scoped parent visible while filtering children", () => {
  const items = buildScopedCommandCenterItems("/agent sp", context) ?? []

  assert.equal(items.some((item) => item.kind === "group" && item.value === "/agent "), true)
  assert.equal(items[0]?.value, "/agent spawn")
})

test("command center scoped items close exact trailing-space leaf commands", () => {
  assert.deepEqual(buildScopedCommandCenterItems("/agent delete ", context), [])
})

test("command center scoped items ignore inputs outside known scopes", () => {
  assert.equal(buildScopedCommandCenterItems("/unknown", context), null)
})

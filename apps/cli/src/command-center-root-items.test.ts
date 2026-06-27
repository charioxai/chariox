import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

import { buildCommandCenterRootItems } from "./command-center-root-items.js"
import type { CommandNode } from "./command-center-tree-projection.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

const commandTree = JSON.parse(readFileSync(
  new URL("../../kernel/src/runtime/terminal_command_catalog/catalog.json", import.meta.url),
  "utf8",
)) as CommandNode[]

test("command center root items include static groups, misc commands, and focused provider namespace", () => {
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

  const items = buildCommandCenterRootItems({
    commandTree,
    focusedProvider: "codex",
    providerCommandCatalogs: catalogs,
  })

  assert.equal(items.some((item) => item.kind === "group" && item.value === "/provider "), true)
  assert.equal(items.some((item) => item.kind === "group" && item.value === "/codex "), true)
  assert.equal(items.some((item) => item.kind === "command" && item.value === "/exit"), true)
  assert.equal(items.find((item) => item.value === "/codex ")?.searchAliases?.includes("resume"), true)
})

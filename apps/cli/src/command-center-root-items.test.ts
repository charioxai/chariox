import assert from "node:assert/strict"
import test from "node:test"

import { buildCommandCenterRootItems } from "./command-center-root-items.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

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
    focusedProvider: "codex",
    providerCommandCatalogs: catalogs,
  })

  assert.equal(items.some((item) => item.kind === "group" && item.value === "/provider "), true)
  assert.equal(items.some((item) => item.kind === "group" && item.value === "/codex "), true)
  assert.equal(items.some((item) => item.kind === "command" && item.value === "/exit"), true)
  assert.equal(items.find((item) => item.value === "/codex ")?.searchAliases?.includes("resume"), true)
})

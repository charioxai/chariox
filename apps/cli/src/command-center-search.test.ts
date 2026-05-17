import assert from "node:assert/strict"
import test from "node:test"

import { filterCommandCenterItems } from "./command-center-search.js"
import type { CommandCenterItem } from "./command-center-types.js"

test("command center search ranks direct matches before aliases and caps results", () => {
  const rankedItems: CommandCenterItem[] = [
    commandItem("alias", "Other alias", "/other", ["launch task"]),
    commandItem("direct", "Launch direct", "/launch"),
  ]
  const cappedItems = Array.from({ length: 25 }, (_, index) => commandItem(`bulk-${index}`, `Bulk ${index}`, `/bulk-${index}`))

  const rankedMatches = filterCommandCenterItems(rankedItems, "launch")
  const cappedMatches = filterCommandCenterItems(cappedItems, "bulk")

  assert.deepEqual(rankedMatches.map((item) => item.id), ["direct", "alias"])
  assert.equal(cappedMatches.length, 20)
})

function commandItem(id: string, label: string, value: string, searchAliases: string[] = []): CommandCenterItem {
  return {
    id,
    label,
    description: `${label} description`,
    kind: "command",
    value,
    searchAliases,
  }
}

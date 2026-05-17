import assert from "node:assert/strict"
import test from "node:test"

import { COMMAND_TREE } from "./command-center-tree.js"
import { collectCommandNodes } from "./command-center-tree-projection.js"

test("command center tree owns unique slash command nodes", () => {
  const nodes = collectCommandNodes(COMMAND_TREE)
  const ids = nodes.map((node) => node.id)

  assert.equal(new Set(ids).size, ids.length)
  assert.equal(COMMAND_TREE.some((node) => node.id === "session"), true)
  assert.equal(COMMAND_TREE.some((node) => node.id === "workflow"), true)
  assert.equal(COMMAND_TREE.find((node) => node.id === "misc")?.value, "/")
})

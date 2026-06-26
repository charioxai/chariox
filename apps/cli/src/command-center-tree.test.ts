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
  assert.equal(COMMAND_TREE.find((node) => node.id === "machine")?.children?.some((node) => node.value === "/machine kernels "), true)
  assert.equal(COMMAND_TREE.find((node) => node.id === "machine")?.children?.some((node) => node.value === "/machine approve "), true)
  assert.equal(COMMAND_TREE.find((node) => node.id === "misc")?.value, "/")
})

test("agent task command tree describes Meta mode task controls", () => {
  const taskNode = collectCommandNodes(COMMAND_TREE).find((node) => node.id === "agent-task")

  assert.equal(taskNode?.description, "View or control the focused agent's Meta mode task")
  assert.equal(taskNode?.searchAliases?.includes("meta mode task"), true)
  assert.equal(taskNode?.searchAliases?.includes("metaagent task"), true)
  assert.equal(taskNode?.children?.find((node) => node.id === "agent-task-plan")?.description, "Edit the Meta mode plan document")
})

import assert from "node:assert/strict"
import test from "node:test"

import {
  collectCommandNodes,
  findDeepestScope,
  mapNodeToItem,
  mapRootGroup,
  type CommandNode,
} from "./command-center-tree-projection.js"

const tree: CommandNode[] = [{
  id: "root",
  label: "/root",
  description: "Root commands",
  value: "/root ",
  children: [{
    id: "child",
    label: "child",
    description: "Child command",
    value: "/root child ",
    children: [{
      id: "leaf",
      label: "leaf",
      description: "Leaf command",
      value: "/root child leaf",
    }],
  }],
}]

test("command tree projection maps groups with descendant aliases", () => {
  const item = mapRootGroup(tree[0]!)

  assert.equal(item.kind, "group")
  assert.equal(item.description, "Root commands (1)")
  assert.equal(item.searchAliases?.includes("/root child"), true)
  assert.equal(item.searchAliases?.includes("Leaf command"), true)
})

test("command tree projection finds deepest scopes and flattens nodes", () => {
  assert.equal(findDeepestScope("/root child value", tree)?.node.id, "child")
  assert.deepEqual(collectCommandNodes(tree).map((node) => node.id), ["root", "child", "leaf"])
  assert.equal(mapNodeToItem(tree[0]!).kind, "group")
  assert.equal(mapNodeToItem(tree[0]!.children![0]!.children![0]!).kind, "command")
})

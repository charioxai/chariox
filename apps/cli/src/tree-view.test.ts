import assert from "node:assert/strict"
import test from "node:test"

import {
  buildDirectoryTreeRows,
  createDirectoryTreeState,
  isDirectoryTreePathLoaded,
  mergeDirectoryTreeEntries,
  moveDirectoryTreeSelection,
  toggleDirectoryTreeExpansion,
} from "./tree-view.js"

test("buildDirectoryTreeRows includes root and nested entries", () => {
  const state = createDirectoryTreeState("/Users/miguel/chariox", [
    { relative_path: "apps", kind: "directory" },
    { relative_path: "apps/cli", kind: "directory" },
    { relative_path: "apps/cli/src", kind: "directory" },
    { relative_path: "apps/cli/src/index.tsx", kind: "file" },
  ])

  assert.deepEqual(
    buildDirectoryTreeRows(state).map((row) => [row.id, row.depth, row.kind]),
    [
      ["", 0, "root"],
      ["apps", 1, "directory"],
    ],
  )
})

test("toggleDirectoryTreeExpansion collapses and re-expands directories", () => {
  let state = createDirectoryTreeState("/Users/miguel/chariox", [
    { relative_path: "apps", kind: "directory" },
    { relative_path: "apps/cli", kind: "directory" },
  ])
  state = moveDirectoryTreeSelection(state, "down")
  state = toggleDirectoryTreeExpansion(state)
  assert.deepEqual(buildDirectoryTreeRows(state).map((row) => row.id), ["", "apps", "apps/cli"])
  state = toggleDirectoryTreeExpansion(state)
  assert.deepEqual(buildDirectoryTreeRows(state).map((row) => row.id), ["", "apps"])
})

test("mergeDirectoryTreeEntries merges a lazily loaded subtree", () => {
  let state = createDirectoryTreeState("/Users/miguel/chariox", [
    { relative_path: "apps", kind: "directory" },
  ])

  state = mergeDirectoryTreeEntries(state, "apps", [
    { relative_path: "cli", kind: "directory" },
    { relative_path: "README.md", kind: "file" },
  ])

  assert.equal(isDirectoryTreePathLoaded(state, "apps"), true)
  assert.deepEqual(
    buildDirectoryTreeRows({ ...state, expandedPaths: ["", "apps"] }).map((row) => row.id),
    ["", "apps", "apps/cli", "apps/README.md"],
  )
})

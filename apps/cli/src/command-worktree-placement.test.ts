import assert from "node:assert/strict"
import test from "node:test"

import { defaultWorktreeDirectoryBase, suggestNamedWorktreePath } from "./command-worktree-placement.js"

test("default worktree directory base uses branch leaf without duplicate repo prefix", () => {
  assert.equal(
    defaultWorktreeDirectoryBase("chariox-cloud", "chariox/chariox-cloud-session-1783622367"),
    "chariox-cloud-session-1783622367",
  )
  assert.equal(
    defaultWorktreeDirectoryBase("chariox", "chariox/chariox-session-1779647319"),
    "chariox-session-1779647319",
  )
  assert.equal(
    defaultWorktreeDirectoryBase("chariox-cloud", "feature/worktree-name"),
    "chariox-cloud-worktree-name",
  )
})

test("suggested worktree path uses de-duplicated default directory base", () => {
  assert.equal(
    suggestNamedWorktreePath("/Users/miguel/chariox-cloud", "chariox/chariox-cloud-session-1783622367"),
    "/Users/miguel/chariox-cloud-session-1783622367",
  )
})

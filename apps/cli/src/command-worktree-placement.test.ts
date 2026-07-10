import assert from "node:assert/strict"
import test from "node:test"

import { defaultWorktreeDirectoryBase, suggestNamedWorktreePath } from "./command-worktree-placement.js"

test("default worktree directory base uses branch leaf without duplicate repo prefix", () => {
  assert.equal(
    defaultWorktreeDirectoryBase("arroba-cloud", "arroba/arroba-cloud-session-1783622367"),
    "arroba-cloud-session-1783622367",
  )
  assert.equal(
    defaultWorktreeDirectoryBase("arroba", "arroba/arroba-session-1779647319"),
    "arroba-session-1779647319",
  )
  assert.equal(
    defaultWorktreeDirectoryBase("arroba-cloud", "feature/worktree-name"),
    "arroba-cloud-worktree-name",
  )
})

test("suggested worktree path uses de-duplicated default directory base", () => {
  assert.equal(
    suggestNamedWorktreePath("/Users/miguel/arroba-cloud", "arroba/arroba-cloud-session-1783622367"),
    "/Users/miguel/arroba-cloud-session-1783622367",
  )
})

import assert from "node:assert/strict"
import test from "node:test"

import {
  formatWorkspaceLiveSyncModeLabel,
  parseWorkspaceLiveSyncModeCommand,
  workspaceLiveSyncModeProtocolValue,
} from "./workspace-live-sync-mode.js"

test("workspace live sync mode helper parses command values", () => {
  assert.equal(parseWorkspaceLiveSyncModeCommand("off"), "off")
  assert.equal(parseWorkspaceLiveSyncModeCommand("managed"), "managed")
  assert.equal(parseWorkspaceLiveSyncModeCommand("tracked"), "tracked")
  assert.equal(parseWorkspaceLiveSyncModeCommand("unrestricted"), null)
  assert.equal(parseWorkspaceLiveSyncModeCommand("on"), null)
})

test("workspace live sync mode helper maps off to protocol unrestricted", () => {
  assert.equal(workspaceLiveSyncModeProtocolValue("off"), "unrestricted")
  assert.equal(workspaceLiveSyncModeProtocolValue("unrestricted"), "unrestricted")
  assert.equal(workspaceLiveSyncModeProtocolValue("managed"), "managed")
  assert.equal(workspaceLiveSyncModeProtocolValue("tracked"), "tracked")
})

test("workspace live sync mode helper formats user-facing scoped labels", () => {
  assert.equal(formatWorkspaceLiveSyncModeLabel("unrestricted"), "off")
  assert.equal(formatWorkspaceLiveSyncModeLabel(null), "config default")
  assert.equal(formatWorkspaceLiveSyncModeLabel(undefined), "config default")
  assert.equal(
    formatWorkspaceLiveSyncModeLabel("managed"),
    "managed (selected workspace/worktree only; other repositories unrestricted)",
  )
  assert.equal(
    formatWorkspaceLiveSyncModeLabel("tracked"),
    "tracked (selected workspace/worktree only; other repositories unrestricted)",
  )
})

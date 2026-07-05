import assert from "node:assert/strict"
import test from "node:test"

import {
  formatWorkspaceLiveSyncModeChangeMessage,
  formatWorkspaceLiveSyncDefaultModeChangeMessage,
  formatWorkspaceLiveSyncModeCompactLabel,
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

test("workspace live sync mode helper formats compact labels", () => {
  assert.equal(formatWorkspaceLiveSyncModeCompactLabel("managed"), "managed")
  assert.equal(formatWorkspaceLiveSyncModeCompactLabel("tracked"), "tracked")
  assert.equal(formatWorkspaceLiveSyncModeCompactLabel("unrestricted"), "off")
  assert.equal(formatWorkspaceLiveSyncModeCompactLabel(null), "off")
  assert.equal(formatWorkspaceLiveSyncModeCompactLabel(undefined), "off")
})

test("workspace live sync mode helper formats scoped mode-change messages", () => {
  assert.equal(
    formatWorkspaceLiveSyncModeChangeMessage("managed"),
    "current session workspace live sync set to managed (selected workspace/worktree only; other repositories unrestricted)",
  )
  assert.equal(
    formatWorkspaceLiveSyncModeChangeMessage("tracked", { action: "enabled" }),
    "current session workspace live sync enabled: tracked (selected workspace/worktree only; other repositories unrestricted)",
  )
  assert.equal(
    formatWorkspaceLiveSyncModeChangeMessage("off"),
    "current session workspace live sync disabled; other repositories remain unrestricted",
  )
  assert.equal(
    formatWorkspaceLiveSyncModeChangeMessage("unrestricted"),
    "current session workspace live sync disabled; other repositories remain unrestricted",
  )
  assert.equal(
    formatWorkspaceLiveSyncModeChangeMessage("tracked", {
      providerReload: { reloaded: 1, deferred: 2, unaffected: 3 },
    }),
    "current session workspace live sync set to tracked (selected workspace/worktree only; other repositories unrestricted); provider reloads: 1 reloaded, 2 deferred, 3 unaffected",
  )
  assert.equal(
    formatWorkspaceLiveSyncModeChangeMessage("off", {
      providerReload: { reloaded: 0, deferred: 0, unaffected: 1 },
    }),
    "current session workspace live sync disabled; other repositories remain unrestricted; provider reloads: none",
  )
})

test("workspace live sync mode helper formats scoped default-change messages", () => {
  assert.equal(
    formatWorkspaceLiveSyncDefaultModeChangeMessage("managed"),
    "default workspace live sync for new sessions set to managed (selected workspace/worktree only; other repositories unrestricted)",
  )
  assert.equal(
    formatWorkspaceLiveSyncDefaultModeChangeMessage("tracked"),
    "default workspace live sync for new sessions set to tracked (selected workspace/worktree only; other repositories unrestricted)",
  )
  assert.equal(
    formatWorkspaceLiveSyncDefaultModeChangeMessage("off"),
    "default workspace live sync for new sessions disabled; other repositories remain unrestricted",
  )
  assert.equal(
    formatWorkspaceLiveSyncDefaultModeChangeMessage("unrestricted"),
    "default workspace live sync for new sessions disabled; other repositories remain unrestricted",
  )
})

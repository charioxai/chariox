import assert from "node:assert/strict"
import test from "node:test"

import { createSessionBrowserProjectionController } from "./session-browser-projection-controller.js"
import type { SessionListEntry } from "./sessions.js"

test("session browser projection filters ended sessions and normalizes selection", () => {
  let selectedIndex = 4
  const controller = createSessionBrowserProjectionController({
    isAttached: () => true,
    availableSessions: () => [
      session("ended", "Ended", 10),
      session("recent", "Created", 30),
      session("old", "Created", 20),
    ],
    selectedIndex: () => selectedIndex,
    setSelectedIndex: (index) => {
      selectedIndex = index
    },
  })

  assert.deepEqual(controller.sessions().map((item) => item.id), ["recent", "old"])
  assert.equal(controller.normalizeIndex(), 1)
  assert.equal(selectedIndex, 1)
})

test("session browser projection switches hotkey sections by attachment state", () => {
  let attached = true
  const controller = createSessionBrowserProjectionController({
    isAttached: () => attached,
    availableSessions: () => [],
    selectedIndex: () => 0,
    setSelectedIndex: () => {},
  })

  assert.deepEqual(controller.hotkeySections().map((section) => section.title), ["Global", "Session"])
  attached = false
  assert.deepEqual(controller.hotkeySections().map((section) => section.title), ["Global", "Waiting room"])
})

function session(id: string, status: string, lastUsedAtMs: number): SessionListEntry {
  return {
    id,
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    last_used_at_ms: lastUsedAtMs,
    status,
    connected_cli_count: 0,
  }
}

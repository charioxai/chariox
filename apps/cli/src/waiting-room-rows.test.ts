import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { waitingRoomRows } from "./waiting-room-rows.js"
import { createWaitingRoomState } from "./waiting-room-state.js"

test("waiting room rows compose start, session, remote, terminal, and theme sections", () => {
  const catalog = fallbackProviderCatalog()
  const sessions: SessionListEntry[] = [{
    id: "session-1",
    alias: null,
    worktree_id: "/workspace/tree",
    status: "Active",
    created_at_ms: Date.UTC(2026, 0, 1, 9, 0),
    last_used_at_ms: Date.UTC(2026, 0, 1, 10, 0),
  }]
  const state = createWaitingRoomState(sessions, catalog, "opencode", "opencode/gpt-5.4", "high")
  const rows = waitingRoomRows(state, sessions, catalog, {
    relay: { configured: true, connected: true, relay_url: "wss://relay.example" },
    terminals: [{ terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 0, revoked: false }],
  })

  assert.equal(rows[0]?.id, "new")
  assert.equal(rows.some((row) => row.id === "session:session-1"), true)
  assert.equal(rows.some((row) => row.id === "relay-header"), true)
  assert.equal(rows.some((row) => row.id === "terminal:terminal-1"), true)
  assert.equal(rows.at(-1)?.id, "theme")
})

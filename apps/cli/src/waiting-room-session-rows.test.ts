import assert from "node:assert/strict"
import test from "node:test"

import type { SessionListEntry } from "./sessions.js"
import {
  MAX_VISIBLE_WAITING_ROOM_SESSIONS,
  waitingRoomMenuMinWidth,
  waitingRoomPreviewSessions,
  waitingRoomSessionRows,
  waitingRoomSessionTitleWidth,
  waitingRoomSessions,
} from "./waiting-room-session-rows.js"

test("waiting room session rows render active sessions with stable columns", () => {
  const sessions = [
    session({ id: "session-old", last_used_at_ms: Date.UTC(2026, 0, 1, 10, 0) }),
    session({
      id: "session-working",
      alias: "frontend",
      status: "Active",
      host_daemon_id: "home-kernel",
      host_machine_id: "home-machine",
      last_used_at_ms: Date.UTC(2026, 0, 2, 10, 0),
      activity: {
        agent_count: 2,
        working_agent_count: 1,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        error_agent_count: 0,
      },
    }),
  ]

  const rows = waitingRoomSessionRows(
    { focus: "session", sessionIndex: 0 },
    sessions,
    { inventoryLoading: false, loadingText: "loading", titleWidth: 32 },
  )

  assert.equal(rows[0]?.id, "session-header")
  assert.equal(rows[1]?.id, "session:session-working")
  assert.equal(rows[1]?.title, "* session-working (frontend)")
  assert.equal(rows[1]?.value, "Working")
  assert.equal(rows[1]?.columns?.[1]?.trim(), "home-kernel@home-machine")
  assert.equal(rows[1]?.columns?.[2]?.trim(), "2026-01-02 10:00 UTC")
  assert.equal(rows[1]?.focused, true)
})

test("waiting room session rows render loading and empty states", () => {
  assert.deepEqual(waitingRoomSessionRows(
    { focus: "join-sessions", sessionIndex: 0 },
    [],
    { inventoryLoading: true, loadingText: "loading..", titleWidth: 24 },
  ).map((row) => row.id), ["sessions-loading"])

  assert.deepEqual(waitingRoomSessionRows(
    { focus: "join-sessions", sessionIndex: 0 },
    [],
    { inventoryLoading: false, loadingText: "loading", titleWidth: 24 },
  ).map((row) => row.id), ["no-sessions"])
})

test("waiting room session helpers filter, sort, and size preview rows", () => {
  const sessions = [
    session({ id: "ended", status: "Ended", last_used_at_ms: Date.UTC(2026, 0, 4) }),
    session({ id: "old", last_used_at_ms: Date.UTC(2026, 0, 1) }),
    session({ id: "new", last_used_at_ms: Date.UTC(2026, 0, 3) }),
    session({ id: "middle", last_used_at_ms: Date.UTC(2026, 0, 2) }),
  ]

  assert.deepEqual(waitingRoomSessions(sessions).map((entry) => entry.id), ["new", "middle", "old"])
  assert.equal(waitingRoomPreviewSessions(sessions).length, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
  assert.equal(waitingRoomSessionTitleWidth(sessions) >= 24, true)
  assert.equal(waitingRoomMenuMinWidth(sessions) > waitingRoomSessionTitleWidth(sessions), true)
})

function session(overrides: Partial<SessionListEntry> = {}): SessionListEntry {
  return {
    id: "session-1",
    alias: null,
    worktree_id: "/workspace/tree",
    status: "Active",
    created_at_ms: Date.UTC(2026, 0, 1, 9, 0),
    last_used_at_ms: Date.UTC(2026, 0, 1, 9, 0),
    ...overrides,
  }
}

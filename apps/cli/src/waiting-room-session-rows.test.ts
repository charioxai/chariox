import assert from "node:assert/strict"
import test from "node:test"

import type { SessionListEntry } from "./sessions.js"
import {
  MAX_VISIBLE_WAITING_ROOM_SESSIONS,
  waitingRoomMenuMinWidth,
  waitingRoomPreviewSessions,
  sessionLastActiveMs,
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
      workspace_live_sync_mode: "managed",
      last_used_at_ms: Date.UTC(2026, 0, 2, 10, 0),
      activity: {
        agent_count: 2,
        working_agent_count: 1,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        error_agent_count: 0,
        remote_agent_count: 1,
        missing_worker_provider_run_count: 1,
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
  assert.equal(rows[1]?.columns?.[2]?.trim(), "managed")
  assert.equal(rows[1]?.columns?.[3]?.trim(), "1 working")
  assert.equal(rows[1]?.columns?.[4]?.trim(), "run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent")
  assert.equal(rows[1]?.columns?.[5]?.trim(), "2026-01-02 10:00 UTC")
  assert.equal(rows[1]?.focused, true)
})

test("waiting room session rows surface active and queued prompt counts", () => {
  const rows = waitingRoomSessionRows(
    { focus: "session", sessionIndex: 0 },
    [
      session({
        id: "session-queued",
        activity: {
          agent_count: 2,
          working_agent_count: 1,
          active_prompt_count: 1,
          queued_prompt_count: 2,
          error_agent_count: 0,
        },
      }),
    ],
    { inventoryLoading: false, loadingText: "loading", titleWidth: 32 },
  )

  assert.equal(rows[0]?.columns?.[3]?.trim(), "Work")
  assert.equal(rows[1]?.value, "Working")
  assert.equal(rows[1]?.columns?.[3]?.trim(), "1 working, 1 active prompt, 2 queued prompts")
})

test("waiting room session rows render done status for unread idle output", () => {
  const sessions = [
    session({
      id: "session-done",
      status: "Active",
      activity: {
        agent_count: 1,
        working_agent_count: 0,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        error_agent_count: 0,
        unread_idle_agent_count: 1,
      },
    }),
  ]

  const rows = waitingRoomSessionRows(
    { focus: "session", sessionIndex: 0 },
    sessions,
    { inventoryLoading: false, loadingText: "loading", titleWidth: 32 },
  )

  assert.equal(rows[1]?.title, "session-done")
  assert.equal(rows[1]?.value, "Done")
  assert.equal(rows[1]?.columns?.[0]?.trim(), "Done")
  assert.equal(rows[1]?.columns?.[3]?.trim(), "-")
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

test("waiting room session rows surface aggregate home-proxy recovery", () => {
  const sessions = [
    session({
      id: "session-tools",
      status: "Active",
      workspace_live_sync_mode: "tracked",
      activity: {
        agent_count: 2,
        working_agent_count: 0,
        active_prompt_count: 0,
        queued_prompt_count: 0,
        error_agent_count: 0,
        remote_agent_count: 1,
        home_proxy_agent_count: 1,
        remote_extension_sync_issue_count: 1,
        remote_extension_pending_revoke_count: 1,
      },
    }),
  ]

  const rows = waitingRoomSessionRows(
    { focus: "session", sessionIndex: 0 },
    sessions,
    { inventoryLoading: false, loadingText: "loading", titleWidth: 32 },
  )

  assert.equal(
    rows[1]?.columns?.[3]?.trim(),
    "-",
  )
  assert.equal(
    rows[1]?.columns?.[4]?.trim(),
    "keep the home revoke in place; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after the worker reconnects",
  )
})

test("waiting room session helpers filter, sort, and size preview rows", () => {
  const sessions = [
    session({ id: "ended", status: "Ended", last_prompt_sent_at_ms: Date.UTC(2026, 0, 6) }),
    session({ id: "prompt", last_prompt_sent_at_ms: Date.UTC(2026, 0, 5) }),
    session({ id: "activity", last_activity_at_ms: Date.UTC(2026, 0, 4) }),
    session({ id: "last-used", last_used_at_ms: Date.UTC(2026, 0, 3) }),
    session({ id: "created", created_at_ms: Date.UTC(2026, 0, 2), last_used_at_ms: null }),
  ]

  assert.deepEqual(waitingRoomSessions(sessions).map((entry) => entry.id), [
    "prompt",
    "activity",
    "last-used",
    "created",
  ])
  assert.equal(waitingRoomPreviewSessions(sessions).length, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
  assert.equal(waitingRoomSessionTitleWidth(sessions) >= 24, true)
  assert.equal(waitingRoomMenuMinWidth(sessions) > waitingRoomSessionTitleWidth(sessions), true)
})

test("waiting room session recency prioritizes prompt and activity timestamps before fallbacks", () => {
  assert.equal(sessionLastActiveMs(session({
    last_prompt_sent_at_ms: 40,
    last_activity_at_ms: 30,
    created_at_ms: 20,
    last_used_at_ms: 10,
  })), 40)
  assert.equal(sessionLastActiveMs(session({
    last_prompt_sent_at_ms: null,
    last_activity_at_ms: 30,
    created_at_ms: 20,
    last_used_at_ms: 10,
  })), 30)
  assert.equal(sessionLastActiveMs(session({
    last_prompt_sent_at_ms: null,
    last_activity_at_ms: null,
    created_at_ms: 20,
    last_used_at_ms: 10,
  })), 10)
  assert.equal(sessionLastActiveMs(session({
    last_prompt_sent_at_ms: null,
    last_activity_at_ms: null,
    created_at_ms: 0,
    last_used_at_ms: 10,
  })), 10)
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

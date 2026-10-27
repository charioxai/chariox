import assert from "node:assert/strict"
import test from "node:test"

import {
  clampSessionBrowserIndex,
  nextSessionBrowserIndex,
  resolveSessionBrowserKeyAction,
  sessionBrowserVisibleSessions,
} from "@chariox/kernel-client/session-browser-policy"

test("resolveSessionBrowserKeyAction ignores inactive or modified events", () => {
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: false,
    event: { name: "enter" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "ignore" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter", eventType: "release" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "ignore" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter", ctrl: true },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "ignore" })
})

test("resolveSessionBrowserKeyAction handles close and movement keys", () => {
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "escape" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "close" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "up" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "move", delta: -1 })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "down" },
    sessionCount: 1,
    selectedIndex: 0,
  }), { action: "move", delta: 1 })
})

test("resolveSessionBrowserKeyAction handles empty, submit, and lifecycle keys", () => {
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter" },
    sessionCount: 0,
    selectedIndex: 0,
  }), { action: "empty" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "enter" },
    sessionCount: 2,
    selectedIndex: 1,
  }), { action: "submit", selectedIndex: 1 })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "a" },
    sessionCount: 2,
    selectedIndex: 1,
  }), { action: "lifecycle", selectedIndex: 1, lifecycleAction: "archive" })
  assert.deepEqual(resolveSessionBrowserKeyAction({
    open: true,
    event: { name: "delete" },
    sessionCount: 2,
    selectedIndex: 1,
  }), { action: "lifecycle", selectedIndex: 1, lifecycleAction: "delete" })
})

test("nextSessionBrowserIndex wraps across available sessions", () => {
  assert.equal(nextSessionBrowserIndex(0, -1, 3), 2)
  assert.equal(nextSessionBrowserIndex(2, 1, 3), 0)
  assert.equal(nextSessionBrowserIndex(2, 1, 0), 2)
})

test("sessionBrowserVisibleSessions filters ended sessions and sorts by recent activity", () => {
  assert.deepEqual(sessionBrowserVisibleSessions([
    session("old", { status: "Created", created_at_ms: 1 }),
    session("ended", { status: "Ended", created_at_ms: 100 }),
    session("recent", { status: "Active", last_used_at_ms: 20, created_at_ms: 2 }),
    session("activity", { status: "Active", last_activity_at_ms: 30, last_used_at_ms: 5, created_at_ms: 3 }),
    session("prompted", { status: "Active", last_prompt_sent_at_ms: 40, last_activity_at_ms: 10, created_at_ms: 4 }),
  ]).map((session) => session.id), ["prompted", "activity", "recent", "old"])
})

test("sessionBrowserVisibleSessions can include ended sessions for archived project drill-down", () => {
  assert.deepEqual(sessionBrowserVisibleSessions([
    session("active", { status: "Active", created_at_ms: 10 }),
    session("ended", { status: "Ended", created_at_ms: 20 }),
  ], { includeEnded: true }).map((item) => item.id), ["ended", "active"])
})

test("clampSessionBrowserIndex keeps selection in range", () => {
  assert.equal(clampSessionBrowserIndex(-1, 3), 0)
  assert.equal(clampSessionBrowserIndex(5, 3), 2)
  assert.equal(clampSessionBrowserIndex(5, 0), 0)
})

function session(id: string, overrides: Partial<Parameters<typeof sessionBrowserVisibleSessions>[0][number]> = {}) {
  return {
    id,
    alias: null,
    worktree_id: "/workspace",
    status: "Created",
    created_at_ms: 1,
    attachment_ids: [],
    ...overrides,
  }
}

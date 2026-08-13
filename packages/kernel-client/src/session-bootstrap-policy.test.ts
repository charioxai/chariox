import assert from "node:assert/strict"
import test from "node:test"

import {
  decideBootstrapAction,
  selectAttachableSession,
} from "./session-bootstrap-policy.js"

test("selectAttachableSession ignores ended sessions and prefers the newest workspace match", () => {
  const selected = selectAttachableSession(
    [
      session({
        id: "deadbeef00000001",
        alias: "old",
        status: "Ended",
        created_at_ms: 99,
      }),
      session({
        id: "deadbeef00000002",
        alias: "keep",
        status: "Parked",
        created_at_ms: 10,
      }),
      session({
        id: "deadbeef00000003",
        alias: "newest",
        status: "Active",
        created_at_ms: 20,
      }),
    ],
    "/Users/miguel/chariox",
    "/Users/miguel/chariox",
  )

  assert.equal(selected?.id, "deadbeef00000003")
})

test("selectAttachableSession requires matching workspace and worktree", () => {
  assert.equal(selectAttachableSession([
    session({ id: "workspace-mismatch", workspace_id: "/tmp/other" }),
    session({ id: "worktree-mismatch", worktree_id: "/tmp/other" }),
  ], "/Users/miguel/chariox", "/Users/miguel/chariox"), null)
})

test("decideBootstrapAction respects explicit create and session refs", () => {
  assert.deepEqual(
    decideBootstrapAction(
      { createSession: true, sessionId: "ignored" },
      [],
      "/Users/miguel/chariox",
      "/Users/miguel/chariox",
    ),
    { action: "create" },
  )
  assert.deepEqual(
    decideBootstrapAction(
      { sessionId: "mai" },
      [session({ id: "deadbeef00000003" })],
      "/Users/miguel/chariox",
      "/Users/miguel/chariox",
    ),
    { action: "resolve", sessionRef: "mai" },
  )
})

test("decideBootstrapAction lands in the waiting room by default", () => {
  assert.deepEqual(
    decideBootstrapAction(
      {},
      [session({ id: "deadbeef00000001", status: "Ended", created_at_ms: 20 })],
      "/Users/miguel/chariox",
      "/Users/miguel/chariox",
    ),
    { action: "none" },
  )
})

test("decideBootstrapAction does not auto-attach existing matching sessions", () => {
  assert.deepEqual(
    decideBootstrapAction(
      {},
      [session({ id: "deadbeef00000003", status: "Active", created_at_ms: 20 })],
      "/Users/miguel/chariox",
      "/Users/miguel/chariox",
    ),
    { action: "none" },
  )
})

function session(overrides: Partial<{
  id: string
  alias: string | null
  workspace_id: string
  worktree_id: string
  status: string
  created_at_ms: number
}> = {}) {
  return {
    id: "deadbeef00000000",
    alias: null,
    workspace_id: "/Users/miguel/chariox",
    worktree_id: "/Users/miguel/chariox",
    status: "Active",
    created_at_ms: 1,
    ...overrides,
  }
}

import assert from "node:assert/strict"
import test from "node:test"

import { decideBootstrapAction, formatSessionList, selectAttachableSession } from "./sessions.js"

test("formatSessionList renders aliases, attachment counts, and current session marker", () => {
  assert.equal(
    formatSessionList(
      [
        {
          id: "session-2",
          alias: "support",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Active",
          created_at_ms: 2,
          attachment_ids: ["attachment-1", "attachment-2"],
        },
        {
          id: "session-1",
          alias: null,
          workspace_id: "/tmp/demo",
          worktree_id: "/tmp/demo",
          status: "Ended",
          created_at_ms: 1,
          attachment_ids: [],
        },
      ],
      "session-2",
    ),
    [
      "Sessions",
      "- `support` (`session-2`) - active - 2 CLIs - arroba current",
      "- `session-1` - ended - 0 CLIs - demo",
    ].join("\n"),
  )
})

test("formatSessionList handles empty session sets", () => {
  assert.equal(formatSessionList([]), "No sessions found.")
})

test("selectAttachableSession ignores ended sessions and prefers the newest workspace match", () => {
  const selected = selectAttachableSession(
    [
      {
        id: "deadbeef00000001",
        alias: "old",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        status: "Ended",
        created_at_ms: 99,
        attachment_ids: [],
      },
      {
        id: "deadbeef00000002",
        alias: "keep",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        status: "Parked",
        created_at_ms: 10,
        attachment_ids: [],
      },
      {
        id: "deadbeef00000003",
        alias: "newest",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        status: "Active",
        created_at_ms: 20,
        attachment_ids: [],
      },
    ],
    "/Users/miguel/arroba",
    "/Users/miguel/arroba",
  )

  assert.equal(selected?.id, "deadbeef00000003")
})

test("decideBootstrapAction respects explicit session refs before the waiting room", () => {
  assert.deepEqual(
    decideBootstrapAction(
      { sessionId: "mai" },
      [
        {
          id: "deadbeef00000003",
          alias: "newest",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Active",
          created_at_ms: 20,
          attachment_ids: [],
        },
      ],
      "/Users/miguel/arroba",
      "/Users/miguel/arroba",
    ),
    { action: "resolve", sessionRef: "mai" },
  )
})

test("decideBootstrapAction lands in the waiting room by default", () => {
  assert.deepEqual(
    decideBootstrapAction(
      {},
      [
        {
          id: "deadbeef00000001",
          alias: "old",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Ended",
          created_at_ms: 20,
          attachment_ids: [],
        },
      ],
      "/Users/miguel/arroba",
      "/Users/miguel/arroba",
    ),
    { action: "none" },
  )
})

test("decideBootstrapAction no longer auto-attaches existing sessions", () => {
  assert.deepEqual(
    decideBootstrapAction(
      {},
      [
        {
          id: "deadbeef00000003",
          alias: "newest",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Active",
          created_at_ms: 20,
          attachment_ids: [],
        },
      ],
      "/Users/miguel/arroba",
      "/Users/miguel/arroba",
    ),
    { action: "none" },
  )
})

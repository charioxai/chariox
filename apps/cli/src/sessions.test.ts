import assert from "node:assert/strict"
import test from "node:test"

import { formatSessionList } from "./sessions.js"

test("formatSessionList renders aliases, attachment counts, and current session marker", () => {
  assert.equal(
    formatSessionList(
      [
        {
          id: "session-2",
          alias: "support",
          worktree_id: "/Users/miguel/arroba",
          status: "Active",
          attachment_ids: ["attachment-1", "attachment-2"],
        },
        {
          id: "session-1",
          alias: null,
          worktree_id: "/tmp/demo",
          status: "Ended",
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

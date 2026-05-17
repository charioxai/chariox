import assert from "node:assert/strict"
import test from "node:test"

import {
  formatWaitingRoomTerminalTitle,
  formatWaitingRoomTerminalType,
  waitingRoomTerminalRows,
  waitingRoomTerminals,
} from "./waiting-room-terminal-rows.js"

test("waiting room terminal rows render terminal metadata and focus", () => {
  const rows = waitingRoomTerminalRows(
    { focus: "terminal", terminalIndex: 1 },
    {
      terminals: [
        {
          terminal_id: "terminal-cli",
          terminal_type: "cli",
          paired_at_ms: 1,
          revoked: false,
        },
        {
          terminal_id: "terminal-web",
          terminal_type: "web",
          alias: "browser",
          paired_at_ms: 1,
          revoked: true,
        },
      ],
    },
    24,
  )

  assert.equal(rows[0]?.id, "terminals-header")
  assert.equal(rows[1]?.columns?.[0]?.trim(), "Type")
  assert.equal(rows[2]?.title, "terminal-cli")
  assert.equal(rows[2]?.value, "CLI")
  assert.equal(rows[2]?.focused, false)
  assert.equal(rows[3]?.title, "terminal-web (browser) (revoked)")
  assert.equal(rows[3]?.value, "Web terminal")
  assert.equal(rows[3]?.focused, true)
  assert.equal(rows[4]?.id, "add-terminal")
})

test("waiting room terminal helpers normalize empty state and labels", () => {
  assert.deepEqual(waitingRoomTerminals({}), [])
  assert.equal(formatWaitingRoomTerminalTitle({
    terminal_id: "terminal-ios",
    terminal_type: "ios",
    paired_at_ms: 1,
    revoked: false,
  }), "terminal-ios")
  assert.equal(formatWaitingRoomTerminalType("android"), "Android terminal")
})

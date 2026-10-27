import assert from "node:assert/strict"
import test from "node:test"

import {
  formatWaitingRoomTerminalTitle,
  formatWaitingRoomTerminalType,
  waitingRoomProjectEnvironmentSetupRows,
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

test("waiting room terminal rows project setup lifecycle and safe failure details", () => {
  const rows = waitingRoomTerminalRows(
    { focus: "terminal", terminalIndex: 0 },
    { terminals: [] },
    24,
    {
      operation_id: "setup/1",
      project_id: "project-1",
      session_id: "session-1",
      agent_id: "agent-1",
      worker_id: "worker-1",
      platform: "linux-x86_64",
      phase: "failed",
      attempt: 1,
      progress_percent: 100,
      definition_digest: null,
      validation: null,
      message: null,
      failure_code: "worker_failed",
      failure_message: "command\nfailed",
      retryable: true,
      created_at_ms: 1,
      updated_at_ms: 2,
    },
  )

  assert.equal(rows.find((row) => row.id === "project-environment-setup-header")?.value, "failed")
  assert.equal(rows.find((row) => row.id === "project-environment-setup:setup-1")?.value, "setup-1 · 100%")
  assert.equal(rows.find((row) => row.id === "project-environment-setup-failure")?.value, "worker_failed: command failed")
})

test("waiting room setup rows preserve existing output when no setup exists", () => {
  assert.deepEqual(waitingRoomProjectEnvironmentSetupRows(null, 24), [])
})

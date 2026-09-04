import assert from "node:assert/strict"
import test from "node:test"

import {
  automationNoticeEntries,
  automationNoticeIds,
  automationNoticeTexts,
  roomActionNoticePattern,
} from "./room-tui-notices.mjs"

test("companion TUI matcher uses the kernel action mode and exact sequence", () => {
  const pattern = roomActionNoticePattern({ sequence: 2, mode: "browser", kind: "click" })
  assert.match("Room action #2: real-codex · browser click · completed", pattern)
  assert.doesNotMatch("Room action #2: real-codex · computer click · completed", pattern)
  assert.doesNotMatch("Room action #3: real-codex · browser click · completed", pattern)
  assert.match("Room action #4: Local user · computer pointer_click · completed",
    roomActionNoticePattern({ sequence: 4, mode: "computer", kind: "pointer_click" }))
  assert.throws(() => roomActionNoticePattern({ sequence: 2, mode: "browser", kind: ".*" }))
})

test("Room TUI notices retain pane-scoped identity across focus changes", () => {
  const snapshot = {
    transcript: {
      visibleAgentId: "agent-1",
      entries: [{ id: 1, role: "notice", text: "focused notice" }],
    },
    agentPanes: {
      "agent-1": [{ id: 1, role: "notice", text: "focused notice" }],
      "agent-2": [
        { id: 1, role: "notice", text: "other pane notice" },
        { id: 2, role: "assistant", text: "not a notice" },
      ],
    },
  }

  assert.deepEqual(automationNoticeEntries(snapshot), [
    { id: "agent-1:1", text: "focused notice" },
    { id: "agent-2:1", text: "other pane notice" },
  ])
  assert.deepEqual(automationNoticeIds(snapshot), ["agent-1:1", "agent-2:1"])
  assert.deepEqual(automationNoticeTexts(snapshot), ["focused notice", "other pane notice"])
})

test("Room TUI notices use authoritative pane identity when the visible transcript has no agent ID", () => {
  const snapshot = {
    transcript: {
      entries: [{ id: 1, role: "notice", text: "focused notice" }],
    },
    agentPanes: {
      "agent-1": [{ id: 1, role: "notice", text: "focused notice" }],
    },
  }

  assert.deepEqual(automationNoticeEntries(snapshot), [
    { id: "agent-1:1", text: "focused notice" },
  ])
})

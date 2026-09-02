import assert from "node:assert/strict"
import test from "node:test"

import {
  automationNoticeEntries,
  automationNoticeIds,
  automationNoticeTexts,
} from "./room-tui-notices.mjs"

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

test("Room TUI notices fall back to the transcript when pane snapshots contain no notices", () => {
  const snapshot = {
    transcript: {
      visibleAgentId: "agent-1",
      entries: [{ id: 1, role: "notice", text: "visible-only notice" }],
    },
    agentPanes: {
      "agent-1": [],
      "agent-2": [{ id: 2, role: "assistant", text: "not a notice" }],
    },
  }

  assert.deepEqual(automationNoticeEntries(snapshot), [
    { id: "agent-1:1", text: "visible-only notice" },
  ])
})

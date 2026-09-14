import assert from "node:assert/strict"

import { assertRetainedClipboardEvidenceIsRedacted } from "./computer-clipboard-x11-drill.mjs"

export function assertRoomClipboardAction(action, { actorId, clipboardText }) {
  assert.ok(action, "missing Room clipboard Action")
  assert.equal(action.actor_id, actorId, "Room clipboard Action actor_id")
  assert.equal(action.mode, "computer", "Room clipboard Action mode")
  assert.equal(action.kind, "clipboard_write", "Room clipboard Action kind")
  assert.equal(action.state, "completed", "Room clipboard Action state")
  assert.equal(action.arguments?.kind, "clipboard_write", "Room clipboard Action arguments kind")
  assert.equal(
    action.arguments?.utf8_byte_count,
    Buffer.byteLength(clipboardText, "utf8"),
    "Room clipboard Action utf8_byte_count",
  )
  assert.equal(
    action.arguments?.character_count,
    [...clipboardText].length,
    "Room clipboard Action character_count",
  )
  assertRetainedClipboardEvidenceIsRedacted(action, clipboardText)
}

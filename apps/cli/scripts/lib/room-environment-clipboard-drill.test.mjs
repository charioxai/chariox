import assert from "node:assert/strict"
import test from "node:test"

import { assertRoomClipboardAction } from "./room-environment-clipboard-drill.mjs"

test("Room clipboard Action retains counts and actor but never text", () => {
  const value = "Clipboard Grüße 世界\n"
  const action = {
    action_id: "action-1",
    actor_id: "user:local",
    mode: "computer",
    kind: "clipboard_write",
    state: "completed",
    arguments: {
      kind: "clipboard_write",
      utf8_byte_count: Buffer.byteLength(value, "utf8"),
      character_count: [...value].length,
    },
  }
  assert.doesNotThrow(() =>
    assertRoomClipboardAction(action, {
      actorId: "user:local",
      clipboardText: value,
    }),
  )
  assert.throws(
    () =>
      assertRoomClipboardAction(
        { ...action, arguments: { ...action.arguments, text: value } },
        { actorId: "user:local", clipboardText: value },
      ),
    /clipboard text/,
  )
})

test("Room clipboard Action validation rejects wrong attribution and counts", () => {
  const value = "hello"
  const action = {
    action_id: "action-2",
    actor_id: "agent:one",
    mode: "computer",
    kind: "clipboard_write",
    state: "completed",
    arguments: {
      kind: "clipboard_write",
      utf8_byte_count: 5,
      character_count: 5,
    },
  }
  assert.throws(
    () =>
      assertRoomClipboardAction(action, {
        actorId: "agent:other",
        clipboardText: value,
      }),
    /actor_id/,
  )
  assert.throws(
    () =>
      assertRoomClipboardAction(
        { ...action, arguments: { ...action.arguments, utf8_byte_count: 4 } },
        { actorId: "agent:one", clipboardText: value },
      ),
    /utf8_byte_count/,
  )
})

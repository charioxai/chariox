import assert from "node:assert/strict"
import test from "node:test"

import {
  assertRoomKeyboardKeyAction,
  assertRoomKeyboardTextAction,
} from "./room-environment-computer-input-drill.mjs"

test("Room keyboard text Action retains counts and attribution without input text", () => {
  const input = "Grüße 世界"
  const action = {
    action_id: "action-keyboard-text",
    actor_id: "agent:one",
    mode: "computer",
    kind: "keyboard_text",
    state: "completed",
    arguments: {
      kind: "keyboard_text",
      utf8_byte_count: 14,
      character_count: 8,
    },
  }

  assert.doesNotThrow(() => assertRoomKeyboardTextAction(action, {
    actorId: "agent:one",
    input,
  }))
  assert.throws(
    () => assertRoomKeyboardTextAction({ ...action, arguments: { ...action.arguments, text: input } }, {
      actorId: "agent:one",
      input,
    }),
    /keyboard input text/,
  )
})

test("Room keyboard key Action retains repeat and attribution without the key", () => {
  const action = {
    action_id: "action-keyboard-key",
    actor_id: "agent:one",
    mode: "computer",
    kind: "keyboard_key",
    state: "completed",
    arguments: {
      kind: "keyboard_key",
      repeat: 3,
    },
  }

  assert.doesNotThrow(() => assertRoomKeyboardKeyAction(action, {
    actorId: "agent:one",
    key: "BackSpace",
    repeat: 3,
  }))
  assert.throws(
    () => assertRoomKeyboardKeyAction({ ...action, arguments: { ...action.arguments, key: "BackSpace" } }, {
      actorId: "agent:one",
      key: "BackSpace",
      repeat: 3,
    }),
    /keyboard key/,
  )
  assert.throws(
    () => assertRoomKeyboardKeyAction(action, {
      actorId: "agent:one",
      key: "BackSpace",
      repeat: 2,
    }),
    /repeat/,
  )
})

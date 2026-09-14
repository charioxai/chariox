import assert from "node:assert/strict"

import { assertRetainedTextIsRedacted } from "./computer-clipboard-x11-drill.mjs"

export function assertRoomKeyboardTextAction(action, { actorId, input }) {
  assertRoomComputerAction(action, { actorId, kind: "keyboard_text" })
  assert.equal(
    action.arguments?.utf8_byte_count,
    Buffer.byteLength(input, "utf8"),
    "Room keyboard text Action utf8_byte_count",
  )
  assert.equal(
    action.arguments?.character_count,
    [...input].length,
    "Room keyboard text Action character_count",
  )
  assertRetainedTextIsRedacted(action, input, "retained evidence contains keyboard input text")
}

export function assertRoomKeyboardKeyAction(action, { actorId, key, repeat }) {
  assertRoomComputerAction(action, { actorId, kind: "keyboard_key" })
  assert.equal(action.arguments?.repeat, repeat, "Room keyboard key Action repeat")
  assertRetainedTextIsRedacted(action, key, "retained evidence contains keyboard key")
}

function assertRoomComputerAction(action, { actorId, kind }) {
  assert.ok(action, `missing Room ${kind} Action`)
  assert.equal(action.actor_id, actorId, `Room ${kind} Action actor_id`)
  assert.equal(action.mode, "computer", `Room ${kind} Action mode`)
  assert.equal(action.kind, kind, `Room ${kind} Action kind`)
  assert.equal(action.state, "completed", `Room ${kind} Action state`)
  assert.equal(action.arguments?.kind, kind, `Room ${kind} Action arguments kind`)
}

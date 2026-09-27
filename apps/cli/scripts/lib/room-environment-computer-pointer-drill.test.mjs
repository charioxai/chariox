import assert from "node:assert/strict"
import test from "node:test"

import {
  assertRoomPointerClickAction,
  assertRoomPointerDragAction,
  assertRoomPointerMoveAction,
  assertRoomPointerScrollAction,
} from "./room-environment-computer-pointer-drill.mjs"

test("Room pointer move Action retains canonical coordinates and attribution", () => {
  const action = {
    action_id: "action-pointer-move",
    actor_id: "agent:one",
    mode: "computer",
    kind: "pointer_move",
    state: "completed",
    arguments: {
      kind: "pointer_move",
      x: 240,
      y: 180,
      viewport_revision: 9,
    },
  }

  assert.doesNotThrow(() => assertRoomPointerMoveAction(action, {
    actorId: "agent:one",
    x: 240,
    y: 180,
    viewportRevision: 9,
  }))
  assert.throws(
    () => assertRoomPointerMoveAction(action, {
      actorId: "agent:one",
      x: 241,
      y: 180,
      viewportRevision: 9,
    }),
    /x/,
  )
})

test("Room pointer click Action distinguishes button and click count", () => {
  const action = {
    action_id: "action-pointer-click",
    actor_id: "agent:one",
    mode: "computer",
    kind: "pointer_click",
    state: "completed",
    arguments: {
      kind: "pointer_click",
      x: 640,
      y: 220,
      button: "right",
      click_count: 2,
      viewport_revision: 9,
    },
  }

  assert.doesNotThrow(() => assertRoomPointerClickAction(action, {
    actorId: "agent:one",
    x: 640,
    y: 220,
    button: "right",
    clickCount: 2,
    viewportRevision: 9,
  }))
  assert.throws(
    () => assertRoomPointerClickAction(action, {
      actorId: "agent:one",
      x: 640,
      y: 220,
      button: "left",
      clickCount: 2,
      viewportRevision: 9,
    }),
    /button/,
  )
  assert.throws(
    () => assertRoomPointerClickAction(action, {
      actorId: "agent:one",
      x: 640,
      y: 220,
      button: "right",
      clickCount: 1,
      viewportRevision: 9,
    }),
    /click_count/,
  )
})

test("Room pointer drag Action retains both endpoints and button", () => {
  const action = {
    action_id: "action-pointer-drag",
    actor_id: "agent:one",
    mode: "computer",
    kind: "pointer_drag",
    state: "completed",
    arguments: {
      kind: "pointer_drag",
      from_x: 200,
      from_y: 360,
      to_x: 920,
      to_y: 360,
      button: "left",
      viewport_revision: 9,
    },
  }

  assert.doesNotThrow(() => assertRoomPointerDragAction(action, {
    actorId: "agent:one",
    fromX: 200,
    fromY: 360,
    toX: 920,
    toY: 360,
    button: "left",
    viewportRevision: 9,
  }))
  assert.throws(
    () => assertRoomPointerDragAction(action, {
      actorId: "agent:one",
      fromX: 200,
      fromY: 360,
      toX: 921,
      toY: 360,
      button: "left",
      viewportRevision: 9,
    }),
    /to_x/,
  )
})

test("Room pointer scroll Action retains anchor and both axes", () => {
  const action = {
    action_id: "action-pointer-scroll",
    actor_id: "agent:one",
    mode: "computer",
    kind: "pointer_scroll",
    state: "completed",
    arguments: {
      kind: "pointer_scroll",
      x: 640,
      y: 650,
      horizontal_steps: 4,
      vertical_steps: 5,
      viewport_revision: 9,
    },
  }

  assert.doesNotThrow(() => assertRoomPointerScrollAction(action, {
    actorId: "agent:one",
    x: 640,
    y: 650,
    horizontalSteps: 4,
    verticalSteps: 5,
    viewportRevision: 9,
  }))
  assert.throws(
    () => assertRoomPointerScrollAction(action, {
      actorId: "agent:one",
      x: 640,
      y: 650,
      horizontalSteps: 3,
      verticalSteps: 5,
      viewportRevision: 9,
    }),
    /horizontal_steps/,
  )
})

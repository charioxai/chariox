import assert from "node:assert/strict"

export function assertRoomPointerMoveAction(action, {
  actorId,
  x,
  y,
  viewportRevision,
}) {
  assertRoomPointerAction(action, { actorId, kind: "pointer_move" })
  assertPointerPosition(action, { label: "move", x, y, viewportRevision })
}

export function assertRoomPointerClickAction(action, {
  actorId,
  x,
  y,
  button,
  clickCount,
  viewportRevision,
}) {
  assertRoomPointerAction(action, { actorId, kind: "pointer_click" })
  assertPointerPosition(action, { label: "click", x, y, viewportRevision })
  assert.equal(action.arguments?.button, button, "Room pointer click Action button")
  assert.equal(
    action.arguments?.click_count,
    clickCount,
    "Room pointer click Action click_count",
  )
}

export function assertRoomPointerDragAction(action, {
  actorId,
  fromX,
  fromY,
  toX,
  toY,
  button,
  viewportRevision,
}) {
  assertRoomPointerAction(action, { actorId, kind: "pointer_drag" })
  assert.equal(action.arguments?.from_x, fromX, "Room pointer drag Action from_x")
  assert.equal(action.arguments?.from_y, fromY, "Room pointer drag Action from_y")
  assert.equal(action.arguments?.to_x, toX, "Room pointer drag Action to_x")
  assert.equal(action.arguments?.to_y, toY, "Room pointer drag Action to_y")
  assert.equal(action.arguments?.button, button, "Room pointer drag Action button")
  assert.equal(
    action.arguments?.viewport_revision,
    viewportRevision,
    "Room pointer drag Action viewport_revision",
  )
}

export function assertRoomPointerScrollAction(action, {
  actorId,
  x,
  y,
  horizontalSteps,
  verticalSteps,
  viewportRevision,
}) {
  assertRoomPointerAction(action, { actorId, kind: "pointer_scroll" })
  assertPointerPosition(action, { label: "scroll", x, y, viewportRevision })
  assert.equal(
    action.arguments?.horizontal_steps,
    horizontalSteps,
    "Room pointer scroll Action horizontal_steps",
  )
  assert.equal(
    action.arguments?.vertical_steps,
    verticalSteps,
    "Room pointer scroll Action vertical_steps",
  )
}

function assertRoomPointerAction(action, { actorId, kind }) {
  assert.ok(action, `missing Room ${kind} Action`)
  assert.equal(action.actor_id, actorId, `Room ${kind} Action actor_id`)
  assert.equal(action.mode, "computer", `Room ${kind} Action mode`)
  assert.equal(action.kind, kind, `Room ${kind} Action kind`)
  assert.equal(action.state, "completed", `Room ${kind} Action state`)
  assert.equal(action.arguments?.kind, kind, `Room ${kind} Action arguments kind`)
}

function assertPointerPosition(action, { label, x, y, viewportRevision }) {
  assert.equal(action.arguments?.x, x, `Room pointer ${label} Action x`)
  assert.equal(action.arguments?.y, y, `Room pointer ${label} Action y`)
  assert.equal(
    action.arguments?.viewport_revision,
    viewportRevision,
    `Room pointer ${label} Action viewport_revision`,
  )
}

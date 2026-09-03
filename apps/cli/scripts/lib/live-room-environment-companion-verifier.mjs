import assert from "node:assert/strict"
import path from "node:path"

import {
  publishRoomDrillCompanionReady,
  waitForRoomDrillCompanionResult,
} from "./room-drill-companion.mjs"

export async function runRoomEnvironmentCompanion(input) {
  const directory = input.env.CHARIOX_ROOM_DRILL_COORDINATION_DIR?.trim()
  if (!directory) return null
  if (!path.isAbsolute(directory)) {
    throw new Error("CHARIOX_ROOM_DRILL_COORDINATION_DIR must be an absolute disposable directory")
  }
  const timeoutMs = companionTimeoutMs(input.env.CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS)
  await input.prepare?.()
  await publishRoomDrillCompanionReady(directory, {
    schema: "chariox.room_environment.companion_ready.v1",
    ...input.ready,
  })
  const companion = await waitForRoomDrillCompanionResult(directory, {
    sessionId: input.ready.sessionId,
    environmentId: input.ready.environmentId,
    timeoutMs,
    pollIntervalMs: 100,
  })
  validateCompanionResult(companion)
  if (input.ready.keyboardText) {
    assert.ok(companion.keyboard, "Web companion omitted required keyboard evidence")
  }
  if (input.ready.keyboardReplacementText) {
    assert.ok(companion.keyboard?.replacement, "Web companion omitted shortcut/IME evidence")
  }
  await input.waitForPhysicalEffect(companion.physicalEffect)
  if (companion.keyboard) {
    assert.equal(companion.keyboard.physicalEffect, "WEB_KEYBOARD_TEXT_OK")
    assert.equal(typeof companion.keyboard.actionId, "string")
    assert.ok(companion.keyboard.actionId.length > 0)
    await input.waitForPhysicalEffect(companion.keyboard.physicalEffect)
    if (companion.keyboard.replacement) {
      assert.equal(companion.keyboard.replacement.physicalEffect, "WEB_KEYBOARD_REPLACEMENT_OK")
      await input.waitForPhysicalEffect(companion.keyboard.replacement.physicalEffect)
    }
  }

  const history = unwrap(
    await input.client.send(input.requests.listRoomEnvironmentActionHistoryRequest(
      input.ready.sessionId,
      null,
      100,
    )),
    "RoomEnvironmentActionHistoryListed",
  ).page.actions
  const webAction = history.find((action) => action.action_id === companion.actionId)
  assert.ok(webAction, `Web companion action ${companion.actionId} was absent from kernel history`)
  assert.equal(webAction.actor_id, companion.actorId)
  assert.equal(webAction.kind, "pointer_click")
  assert.equal(webAction.state, "completed")
  const actions = [webAction]
  if (companion.keyboard) {
    const keyboard = history.find((action) => action.action_id === companion.keyboard.actionId)
    assert.ok(keyboard, "Web keyboard action was absent from kernel history")
    assert.equal(keyboard.kind, "keyboard_text")
    assert.equal(keyboard.state, "completed")
    assert.equal(keyboard.actor_id, companion.actorId)
    assert.ok(keyboard.sequence > webAction.sequence, "typing must follow the focus click")
    if (input.ready.keyboardText) {
      assert.ok(!JSON.stringify(history).includes(input.ready.keyboardText), "history retained Web typed text")
    }
    actions.push(keyboard)
    if (companion.keyboard.replacement) {
      const replacement = companion.keyboard.replacement
      let previous = keyboard
      for (const [id, kind] of [[replacement.shortcutActionId, "keyboard_key"], [replacement.actionId, "keyboard_text"]]) {
        assert.equal(typeof id, "string")
        assert.ok(id.length > 0)
        const action = history.find((item) => item.action_id === id)
        assert.ok(action, "Web shortcut/IME action was absent from kernel history")
        assert.equal(action.kind, kind)
        assert.equal(action.state, "completed")
        assert.equal(action.actor_id, companion.actorId)
        assert.ok(action.sequence > previous.sequence, "shortcut and IME must follow initial typing in order")
        actions.push(action)
        previous = action
      }
      if (input.ready.keyboardReplacementText) {
        assert.ok(!JSON.stringify(history).includes(input.ready.keyboardReplacementText), "history retained Web IME text")
      }
    }
  }

  await input.activityController.synchronize()
  for (const action of actions) {
    await Promise.all([
      input.waitForLocalActionNotice(input.localNoticeIds, action),
      input.waitForRemoteActionNotice(input.remoteNoticeIds, action),
    ])
  }
  const after = unwrap(
    await input.observerClient.send(input.requests.getRoomEnvironmentStateRequest(input.ready.sessionId)),
    "RoomEnvironmentState",
  ).environment
  assert.equal(after.input_ownership.some((owner) => owner.target?.kind === "desktop"), false)
  return companion
}

function validateCompanionResult(companion) {
  assert.equal(companion.status, "passed", "companion status must be passed")
  assert.ok(
    typeof companion.client === "string" && companion.client.trim().length > 0,
    "companion client must be a non-empty string",
  )
  assert.equal(typeof companion.actionId, "string")
  assert.ok(companion.actionId.length > 0)
  assert.equal(typeof companion.actorId, "string")
  assert.ok(companion.actorId.length > 0)
  assert.match(companion.physicalEffect, /^POINTER_CLICK_COUNT=\d+$/)
  assert.ok(
    typeof companion.screenshot === "string" && path.isAbsolute(companion.screenshot),
    "companion screenshot must be an absolute path",
  )
}

function companionTimeoutMs(value) {
  if (value === undefined || value.trim() === "") return 180_000
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed <= 0 || parsed > 600_000) {
    throw new Error("CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS must be an integer from 1 to 600000")
  }
  return parsed
}

function unwrap(response, variant) {
  assert.ok(response && typeof response === "object" && variant in response, `expected ${variant}, got ${JSON.stringify(response)}`)
  return response[variant]
}

import assert from "node:assert/strict"
import { access, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { runRoomEnvironmentCompanion } from "./live-room-environment-companion-verifier.mjs"

test("Room companion verifier uses stable TUI notice baselines", async () => {
  let prepared = false
  let preparedAtReady = false
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-verifier-"))
  const localNoticeIds = [1]
  const remoteNoticeIds = [2]
  const action = {
    sequence: 7,
    action_id: "action-web",
    actor_id: "user:local",
    kind: "pointer_click",
    state: "completed",
  }
  const keyboardAction = { ...action, action_id: "action-keyboard", kind: "keyboard_text", sequence: 8 }
  const shortcutAction = { ...action, action_id: "action-shortcut", kind: "keyboard_key", sequence: 9 }
  const replacementAction = { ...keyboardAction, action_id: "action-ime", sequence: 10 }
  const noticed = { local: [], remote: [] }
  const physical = []
  const resultWriter = (async () => {
    const readyPath = path.join(root, "ready.json")
    while (true) {
      try {
        await access(readyPath)
        break
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 5))
      }
    }
    preparedAtReady = prepared
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "passed",
      sessionId: "session-1",
      environmentId: "environment-1",
      actionId: action.action_id,
      actorId: action.actor_id,
      keyboard: {
        actionId: keyboardAction.action_id, physicalEffect: "WEB_KEYBOARD_TEXT_OK",
        replacement: {
          shortcutActionId: shortcutAction.action_id,
          actionId: replacementAction.action_id,
          physicalEffect: "WEB_KEYBOARD_REPLACEMENT_OK",
        },
      },
      physicalEffect: "POINTER_CLICK_COUNT=2",
      client: "production-local-web-view",
      screenshot: path.join(root, "web-room-tui-shared.png"),
    }))
  })()

  try {
    const verified = await runRoomEnvironmentCompanion({
      prepare: async () => { prepared = true },
      env: {
        CHARIOX_ROOM_DRILL_COORDINATION_DIR: root,
        CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000",
      },
      ready: {
        schema: "chariox.room_environment.companion_ready.v1",
        sessionId: "session-1",
        environmentId: "environment-1",
        keyboardText: "fixture typing",
        keyboardReplacementText: "fixture replacement",
      },
      client: {
        send: async () => ({ RoomEnvironmentActionHistoryListed: { page: { actions: [action, keyboardAction, shortcutAction, replacementAction] } } }),
      },
      observerClient: {
        send: async () => ({ RoomEnvironmentState: { environment: { input_ownership: [] } } }),
      },
      requests: {
        listRoomEnvironmentActionHistoryRequest: () => ({}),
        getRoomEnvironmentStateRequest: () => ({}),
      },
      activityController: { synchronize: async () => true },
      localNoticeIds,
      remoteNoticeIds,
      waitForPhysicalEffect: async (value) => { physical.push(value) },
      waitForLocalActionNotice: async (baseline, target) => {
        assert.equal(baseline, localNoticeIds)
        noticed.local.push(target?.sequence)
      },
      waitForRemoteActionNotice: async (baseline, target) => {
        assert.equal(baseline, remoteNoticeIds)
        noticed.remote.push(target?.sequence)
      },
    })

    assert.equal(verified.actionId, action.action_id)
    assert.equal(verified.status, "passed")
    assert.deepEqual(physical, ["POINTER_CLICK_COUNT=2", "WEB_KEYBOARD_TEXT_OK", "WEB_KEYBOARD_REPLACEMENT_OK"])
    assert.deepEqual(noticed, { local: [7, 8, 9, 10], remote: [7, 8, 9, 10] })
    assert.equal(preparedAtReady, true, "physical fixture must be reset before Web receives its handoff")
    assert.equal(verified.client, "production-local-web-view")
    assert.equal(verified.screenshot, path.join(root, "web-room-tui-shared.png"))
    await resultWriter
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Room companion verifier rejects incomplete evidence metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-verifier-"))
  const resultWriter = (async () => {
    const readyPath = path.join(root, "ready.json")
    while (true) {
      try {
        await access(readyPath)
        break
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 5))
      }
    }
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "passed",
      sessionId: "session-1",
      environmentId: "environment-1",
      actionId: "action-web",
      actorId: "user:local",
      physicalEffect: "POINTER_CLICK_COUNT=2",
    }))
  })()

  try {
    await assert.rejects(runRoomEnvironmentCompanion({
      env: {
        CHARIOX_ROOM_DRILL_COORDINATION_DIR: root,
        CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000",
      },
      ready: {
        schema: "chariox.room_environment.companion_ready.v1",
        sessionId: "session-1",
        environmentId: "environment-1",
      },
      waitForPhysicalEffect: async () => undefined,
    }), /companion client/i)
    await resultWriter
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

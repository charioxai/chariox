import assert from "node:assert/strict"
import { access, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { runRoomEnvironmentCompanion } from "./live-room-environment-companion-verifier.mjs"

test("Room companion verifier uses stable TUI notice baselines", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-verifier-"))
  const localNoticeIds = [1]
  const remoteNoticeIds = [2]
  const action = {
    action_id: "action-web",
    actor_id: "user:local",
    kind: "pointer_click",
    state: "completed",
  }
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
      actionId: action.action_id,
      actorId: action.actor_id,
      physicalEffect: "POINTER_CLICK_COUNT=2",
    }))
  })()

  try {
    const verified = await runRoomEnvironmentCompanion({
      env: {
        CHARIOX_ROOM_DRILL_COORDINATION_DIR: root,
        CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS: "1000",
      },
      ready: {
        schema: "chariox.room_environment.companion_ready.v1",
        sessionId: "session-1",
        environmentId: "environment-1",
      },
      client: {
        send: async () => ({ RoomEnvironmentActionHistoryListed: { page: { actions: [action] } } }),
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
      waitForPhysicalEffect: async (value) => assert.equal(value, "POINTER_CLICK_COUNT=2"),
      waitForLocalActionNotice: async (baseline) => assert.equal(baseline, localNoticeIds),
      waitForRemoteActionNotice: async (baseline) => assert.equal(baseline, remoteNoticeIds),
    })

    assert.equal(verified.actionId, action.action_id)
    await resultWriter
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

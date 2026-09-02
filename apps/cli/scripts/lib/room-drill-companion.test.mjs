import assert from "node:assert/strict"
import { access, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  publishRoomDrillCompanionReady,
  waitForRoomDrillCompanionResult,
} from "./room-drill-companion.mjs"

test("Room drill companion handoff is private and validates the matching result", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-"))
  try {
    const ready = {
      schema: "chariox.room_environment.companion_ready.v1",
      sessionId: "session-1",
      environmentId: "environment-1",
      relayToken: "secret-token",
    }
    const readyPath = await publishRoomDrillCompanionReady(root, ready)
    assert.equal((await stat(readyPath)).mode & 0o777, 0o600)
    assert.deepEqual(JSON.parse(await readFile(readyPath, "utf8")), ready)

    const resultPromise = waitForRoomDrillCompanionResult(root, {
      sessionId: "session-1",
      environmentId: "environment-1",
      timeoutMs: 1_000,
      pollIntervalMs: 10,
    })
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "passed",
      sessionId: "session-1",
      environmentId: "environment-1",
      actionId: "action-1",
    }))

    assert.equal((await resultPromise).actionId, "action-1")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Room drill companion retries a valid but incomplete result write", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-"))
  try {
    const resultPath = path.join(root, "result.json")
    await writeFile(resultPath, JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
    }))
    const resultPromise = waitForRoomDrillCompanionResult(root, {
      sessionId: "session-1",
      environmentId: "environment-1",
      timeoutMs: 1_000,
      pollIntervalMs: 5,
    })
    await new Promise((resolve) => setTimeout(resolve, 20))
    await writeFile(resultPath, JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "passed",
      sessionId: "session-1",
      environmentId: "environment-1",
      actionId: "action-1",
    }))

    assert.equal((await resultPromise).actionId, "action-1")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Room drill companion rejects a stale or failed result", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-"))
  try {
    await writeFile(path.join(root, "result.json"), JSON.stringify({
      schema: "chariox.room_environment.companion_result.v1",
      status: "failed",
      sessionId: "other-session",
      environmentId: "environment-1",
      error: "browser failed",
    }))

    await assert.rejects(
      waitForRoomDrillCompanionResult(root, {
        sessionId: "session-1",
        environmentId: "environment-1",
        timeoutMs: 100,
        pollIntervalMs: 10,
      }),
      /session mismatch/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Room drill companion times out without leaving a result file", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-room-companion-"))
  try {
    await assert.rejects(
      waitForRoomDrillCompanionResult(root, {
        sessionId: "session-1",
        environmentId: "environment-1",
        timeoutMs: 30,
        pollIntervalMs: 5,
      }),
      /timed out/,
    )
    await assert.rejects(access(path.join(root, "result.json")))
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

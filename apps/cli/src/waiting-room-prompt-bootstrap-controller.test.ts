import assert from "node:assert/strict"
import test from "node:test"

import {
  createWaitingRoomPromptBootstrapController,
} from "./waiting-room-prompt-bootstrap-controller.js"

test("waiting-room prompt bootstrap starts one session and reports bootstrapped", async () => {
  let starts = 0
  const controller = createWaitingRoomPromptBootstrapController({
    isAttached: () => false,
    startSessionFromWaitingRoomDefaults: async () => {
      starts += 1
    },
    flashFooter: () => {},
  })

  assert.equal(await controller.bootstrap(), "bootstrapped")
  assert.equal(starts, 1)
})

test("waiting-room prompt bootstrap guards duplicate submits while pending", async () => {
  let resolveStart!: () => void
  const startBlocked = new Promise<void>((resolve) => {
    resolveStart = resolve
  })
  let starts = 0
  const flashes: string[] = []
  const controller = createWaitingRoomPromptBootstrapController({
    isAttached: () => false,
    startSessionFromWaitingRoomDefaults: async () => {
      starts += 1
      await startBlocked
    },
    flashFooter: (message) => {
      flashes.push(message)
    },
  })

  const first = controller.bootstrap()
  assert.equal(await controller.bootstrap(), "handled")
  resolveStart()
  assert.equal(await first, "bootstrapped")
  assert.equal(starts, 1)
  assert.deepEqual(flashes, ["starting session"])
})

test("waiting-room prompt bootstrap handles start failures without throwing", async () => {
  const flashes: string[] = []
  const warnings: Array<Record<string, unknown>> = []
  const controller = createWaitingRoomPromptBootstrapController({
    isAttached: () => false,
    startSessionFromWaitingRoomDefaults: async () => {
      throw new Error("create failed")
    },
    flashFooter: (message) => {
      flashes.push(message)
    },
    warn: (_message, fields) => {
      warnings.push(fields)
    },
  })

  assert.equal(await controller.bootstrap(), "handled")
  assert.deepEqual(flashes, ["create failed"])
  assert.deepEqual(warnings, [{ error: "create failed" }])
})

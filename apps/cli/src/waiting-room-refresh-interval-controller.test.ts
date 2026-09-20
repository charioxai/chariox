import assert from "node:assert/strict"
import test from "node:test"

import {
  setActiveProjectEnvironmentSetupProjection,
  type ProjectEnvironmentSetupProjection,
} from "./project-environment-setup-projection.js"
import { createWaitingRoomRefreshIntervalController } from "./waiting-room-refresh-interval-controller.js"

test("waiting room refresh interval delegates tick to inventory refresh", () => {
  const harness = createHarness()

  harness.controller.tick()

  assert.deepEqual(harness.calls(), ["refresh"])
})

test("waiting room refresh interval polls the active project setup without replacing inventory refresh", () => {
  const calls: string[] = []
  const projection = {
    poll: async () => {
      calls.push("setup")
      return null
    },
  } as ProjectEnvironmentSetupProjection
  const dispose = setActiveProjectEnvironmentSetupProjection(projection)
  try {
    createWaitingRoomRefreshIntervalController<string>({
      intervalMs: 2_500,
      scheduleInterval: () => "timer-1",
      clearInterval: () => {},
      refreshWaitingRoomData: () => {
        calls.push("refresh")
      },
    }).tick()

    assert.deepEqual(calls, ["refresh", "setup"])
  } finally {
    dispose()
  }
})

test("waiting room refresh interval starts and stops one interval", () => {
  const harness = createHarness()

  harness.controller.start()
  harness.controller.start()
  harness.controller.stop()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), ["interval:2500", "clear:timer-1"])
})

function createHarness() {
  const calls: string[] = []
  const controller = createWaitingRoomRefreshIntervalController<string>({
    intervalMs: 2_500,
    scheduleInterval: (_callback, intervalMs) => {
      calls.push(`interval:${intervalMs}`)
      return "timer-1"
    },
    clearInterval: (handle) => {
      calls.push(`clear:${handle}`)
    },
    refreshWaitingRoomData: () => {
      calls.push("refresh")
    },
  })

  return {
    controller,
    calls: () => calls,
  }
}

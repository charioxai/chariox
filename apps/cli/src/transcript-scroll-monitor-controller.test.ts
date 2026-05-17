import assert from "node:assert/strict"
import test from "node:test"

import { createTranscriptScrollMonitorController } from "./transcript-scroll-monitor-controller.js"

test("transcript scroll monitor delegates tick to history autoload", () => {
  const harness = createHarness()

  harness.controller.tick()

  assert.deepEqual(harness.calls(), ["monitor"])
})

test("transcript scroll monitor starts and stops one interval", () => {
  const harness = createHarness()

  harness.controller.start()
  harness.controller.start()
  harness.controller.stop()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), ["interval:75", "clear:timer-1"])
})

function createHarness() {
  const calls: string[] = []
  const controller = createTranscriptScrollMonitorController<string>({
    intervalMs: 75,
    scheduleInterval: (_callback, intervalMs) => {
      calls.push(`interval:${intervalMs}`)
      return "timer-1"
    },
    clearInterval: (handle) => {
      calls.push(`clear:${handle}`)
    },
    monitorScroll: () => {
      calls.push("monitor")
    },
  })

  return {
    controller,
    calls: () => calls,
  }
}

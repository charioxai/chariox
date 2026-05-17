import assert from "node:assert/strict"
import test from "node:test"

import { createDaemonActivityController } from "./daemon-activity-controller.js"

test("daemon activity records connection activity without repainting healthy state", () => {
  const calls: string[] = []
  const controller = createDaemonActivityController({
    recordConnectionActivity: () => calls.push("activity"),
    daemonDisconnected: () => false,
    setDaemonDisconnected: (disconnected) => calls.push(`disconnected:${disconnected}`),
    updateSessionChrome: () => calls.push("chrome"),
  })

  controller.record("poll")

  assert.deepEqual(calls, ["activity"])
})

test("daemon activity clears stale disconnected state once activity resumes", () => {
  const calls: string[] = []
  const controller = createDaemonActivityController({
    recordConnectionActivity: () => calls.push("activity"),
    daemonDisconnected: () => true,
    setDaemonDisconnected: (disconnected) => calls.push(`disconnected:${disconnected}`),
    updateSessionChrome: () => calls.push("chrome"),
  })

  controller.record("kernel_event")

  assert.deepEqual(calls, ["activity", "disconnected:false", "chrome"])
})

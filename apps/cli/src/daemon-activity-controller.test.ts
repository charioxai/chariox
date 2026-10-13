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

test("MP-08/MP-10 fresh kernel activity invokes shared connection recovery", () => {
  let disconnected = true
  let recovered = 0
  const dependencies = {
    recordConnectionActivity: () => {}, daemonDisconnected: () => disconnected,
    setDaemonDisconnected: (value: boolean) => { disconnected = value }, updateSessionChrome: () => {},
    onRecovered: () => { recovered++ },
  }
  createDaemonActivityController(dependencies).record("heartbeat")
  assert.equal(disconnected, false)
  assert.equal(recovered, 1)
})

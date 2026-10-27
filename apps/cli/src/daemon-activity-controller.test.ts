import assert from "node:assert/strict"
import test from "node:test"

import { createKernelConnectionDisplay } from "./kernel-connection-display.js"
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


test("MP-08/MP-10 control activity cannot reset a session binding repair deadline", () => {
  let disconnected = true
  let repairing = true
  const statuses: string[] = []
  const timers = new Map<number, () => void>()
  let next = 0
  const display = createKernelConnectionDisplay({
    daemonDisconnected: () => disconnected,
    setStatusLine: (s: string) => statuses.push(s), updateSessionChrome: () => {}, connectedStatusLine: "Connected",
    timer: { setTimeout: (callback: () => void) => { timers.set(++next, callback); return next }, clearTimeout: (id: number) => { timers.delete(id) } },
  } as never)
  const activity = createDaemonActivityController({
    recordConnectionActivity: () => {}, daemonDisconnected: () => disconnected,
    setDaemonDisconnected: (value: boolean) => { disconnected = value }, updateSessionChrome: () => {},
    canRecoverConnection: () => !repairing,
    onRecovered: () => display.resumed(),
  } as never)
  display.closed()
  activity.record("control heartbeat")
  assert.equal(disconnected, true)
  assert.deepEqual(statuses, ["Reconnecting to the Chariox kernel…"])
  display.closed()
  assert.equal(next, 1, "failed resubscribe keeps the original deadline")
  timers.get(1)!()
  assert.match(statuses.at(-1)!, /^Disconnected/)
  repairing = false
  activity.record("binding recovered")
  assert.equal(disconnected, false)
  assert.equal(statuses.at(-1), "Connected")
})

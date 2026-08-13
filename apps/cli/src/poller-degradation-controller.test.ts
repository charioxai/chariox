import assert from "node:assert/strict"
import test from "node:test"

import { createPollerDegradationController } from "./poller-degradation-controller.js"

test("poller degradation controller marks the first degraded poller as disconnected", () => {
  const harness = createHarness()

  harness.controller.markDegraded("polling terminal output", "Waiting to reconnect.")

  assert.deepEqual(harness.controller.degradedOperations(), ["polling terminal output"])
  assert.deepEqual(harness.calls(), [
    "disconnected:true",
    "warn:poller entered degraded mode:polling terminal output",
    "status:Waiting to reconnect.",
    "chrome",
    "notice:Waiting to reconnect.:warning",
  ])
})

test("poller degradation controller does not repeat notices for additional degraded pollers", () => {
  const harness = createHarness()

  harness.controller.markDegraded("polling terminal output", "Waiting to reconnect.")
  harness.controller.markDegraded("polling session state", "Still degraded.")

  assert.deepEqual(harness.controller.degradedOperations(), [
    "polling terminal output",
    "polling session state",
  ])
  assert.deepEqual(harness.calls(), [
    "disconnected:true",
    "warn:poller entered degraded mode:polling terminal output",
    "status:Waiting to reconnect.",
    "chrome",
    "notice:Waiting to reconnect.:warning",
    "disconnected:true",
    "warn:poller entered degraded mode:polling session state",
    "status:Still degraded.",
    "chrome",
  ])
})

test("poller degradation controller ignores zero-failure recovery callbacks", () => {
  const harness = createHarness()

  harness.controller.markRecovered("polling terminal output", 0)

  assert.deepEqual(harness.calls(), [])
})

test("poller degradation controller waits for all degraded pollers before marking recovered", () => {
  const harness = createHarness()

  harness.controller.markDegraded("polling terminal output", "Waiting to reconnect.")
  harness.controller.markDegraded("polling session state", "Still degraded.")
  harness.clearCalls()
  harness.controller.markRecovered("polling terminal output", 2)

  assert.deepEqual(harness.controller.degradedOperations(), ["polling session state"])
  assert.deepEqual(harness.calls(), [
    "info:poller recovered:polling terminal output:2",
  ])
})

test("poller degradation controller clears disconnected state after the last degraded poller recovers", () => {
  const harness = createHarness()

  harness.controller.markDegraded("polling terminal output", "Waiting to reconnect.")
  harness.clearCalls()
  harness.controller.markRecovered("polling terminal output", 3)

  assert.deepEqual(harness.controller.degradedOperations(), [])
  assert.deepEqual(harness.calls(), [
    "info:poller recovered:polling terminal output:3",
    "disconnected:false",
    "status:Connected to Chariox daemon.",
    "chrome",
    "notice:Reconnected to the Chariox daemon.:default",
  ])
})

function createHarness() {
  const calls: string[] = []
  const controller = createPollerDegradationController({
    connectedStatusLine: "Connected to Chariox daemon.",
    logger: {
      warn: (message, fields) => {
        calls.push(`warn:${message}:${String(fields?.operation)}`)
      },
      info: (message, fields) => {
        calls.push(`info:${message}:${String(fields?.operation)}:${String(fields?.prior_failures)}`)
      },
    },
    setDaemonDisconnected: (value) => {
      calls.push(`disconnected:${value}`)
    },
    setStatusLine: (value) => {
      calls.push(`status:${value}`)
    },
    updateSessionChrome: () => {
      calls.push("chrome")
    },
    appendNotice: (message, tone) => {
      calls.push(`notice:${message}:${tone ?? "default"}`)
    },
  })

  return {
    controller,
    calls: () => calls,
    clearCalls: () => {
      calls.length = 0
    },
  }
}

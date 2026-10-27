import assert from "node:assert/strict"
import test from "node:test"

import { createPollerDegradationController } from "./poller-degradation-controller.js"

test("MP-08/MP-10 poller degradation keeps transport status out of agent output", () => {
  const harness = createHarness()

  harness.controller.markDegraded("polling terminal output", "Waiting to reconnect.")

  assert.deepEqual(harness.controller.degradedOperations(), ["polling terminal output"])
  assert.deepEqual(harness.calls(), [
    "warn:poller entered degraded mode:polling terminal output",
    "disconnected:true",
    "status:Waiting to reconnect.",
    "chrome",
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
    "warn:poller entered degraded mode:polling terminal output",
    "disconnected:true",
    "status:Waiting to reconnect.",
    "chrome",
    "warn:poller entered degraded mode:polling session state",
    "disconnected:true",
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
  ])
})

function createHarness() {
  const calls: string[] = []
  const dependencies = {
    logger: {
      warn: (message: string, fields?: Record<string, unknown>) => {
        calls.push(`warn:${message}:${String(fields?.operation)}`)
      },
      info: (message: string, fields?: Record<string, unknown>) => {
        calls.push(`info:${message}:${String(fields?.operation)}:${String(fields?.prior_failures)}`)
      },
    },
    onTransportClosed: (message: string) => {
      calls.push("disconnected:true", `status:${message}`, "chrome")
    },
    onTransportRecovered: () => {
      calls.push("disconnected:false", "status:Connected to Chariox daemon.", "chrome")
    },
  }
  const controller = createPollerDegradationController(dependencies)

  return {
    controller,
    calls: () => calls,
    clearCalls: () => {
      calls.length = 0
    },
  }
}

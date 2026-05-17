import assert from "node:assert/strict"
import test from "node:test"

import { createProviderActivityController } from "./provider-activity-controller.js"

test("provider activity controller marks the app working for active provider output", () => {
  const harness = providerActivityHarness()

  harness.controller.apply(true)

  assert.deepEqual(harness.calls, [
    "working:true",
    "provider:true",
    "chrome",
  ])
})

test("provider activity controller keeps working state unchanged for inactive provider output", () => {
  const harness = providerActivityHarness()

  harness.controller.apply(false)

  assert.deepEqual(harness.calls, [
    "provider:false",
    "chrome",
  ])
})

function providerActivityHarness() {
  const harness = {
    calls: [] as string[],
    controller: null as ReturnType<typeof createProviderActivityController> | null,
  }
  harness.controller = createProviderActivityController({
    setWorking: (working) => {
      harness.calls.push(`working:${String(working)}`)
    },
    handleProviderActivity: (active) => {
      harness.calls.push(`provider:${String(active)}`)
    },
    updateSessionChrome: () => {
      harness.calls.push("chrome")
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createProviderActivityController>
  }
}

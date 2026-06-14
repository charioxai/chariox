import assert from "node:assert/strict"
import test from "node:test"

import { waitForCondition } from "./drill-runtime-helpers.mjs"

test("waitForCondition returns the first ready observation", async () => {
  let count = 0
  const observed = await waitForCondition({
    label: "counter",
    timeoutMs: 100,
    pollMs: 1,
    observe: async () => ({ count: ++count }),
    isReady: (value) => value.count === 2,
  })

  assert.deepEqual(observed, { count: 2 })
})

test("waitForCondition reports the last observation on timeout", async () => {
  await assert.rejects(
    () => waitForCondition({
      label: "agent idle",
      timeoutMs: 5,
      pollMs: 1,
      observe: async () => ({ agent: "agent-1", state: "Working" }),
      isReady: () => false,
    }),
    /timed out waiting for agent idle\nlast_observation=/,
  )
})

test("waitForCondition reports transient observer errors", async () => {
  await assert.rejects(
    () => waitForCondition({
      label: "relay freshness",
      timeoutMs: 5,
      pollMs: 1,
      observe: async () => {
        throw new Error("relay unavailable")
      },
    }),
    /last_error=Error: relay unavailable/,
  )
})

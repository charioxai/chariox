import assert from "node:assert/strict"
import test from "node:test"

import { assertOnlySlowSubscriptionClosed } from "./reconnect-storm-pressure.mjs"

test("reconnect storm requires every healthy subscription after slow-lane isolation", () => {
  assert.doesNotThrow(() => assertOnlySlowSubscriptionClosed({ subscription_count: 31 }, 32))
  assert.throws(
    () => assertOnlySlowSubscriptionClosed({ subscription_count: 30 }, 32),
    /retain all 31 healthy subscribers/,
  )
})

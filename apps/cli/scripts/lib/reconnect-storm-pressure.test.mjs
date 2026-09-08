import assert from "node:assert/strict"
import test from "node:test"

import { assertOnlySlowSubscriptionClosed } from "./reconnect-storm-pressure.mjs"

test("reconnect storm requires every healthy subscription after slow-lane isolation", () => {
  const marker = "post-close"
  const healthySeen = Array.from({ length: 31 }, () => new Set([marker]))
  assert.doesNotThrow(() => assertOnlySlowSubscriptionClosed({ subscription_count: 31 }, 32, healthySeen, marker))
  assert.throws(
    () => assertOnlySlowSubscriptionClosed({ subscription_count: 30 }, 32, healthySeen, marker),
    /retain all 31 healthy subscribers/,
  )
})

test("correct count cannot hide an evicted healthy subscriber", () => {
  const marker = "post-close"
  const healthySeen = [new Set([marker]), new Set(["before-close"])]
  assert.throws(
    () => assertOnlySlowSubscriptionClosed({ subscription_count: 2 }, 3, healthySeen, marker),
    /healthy subscriber 2 did not receive the post-close marker/,
  )
})

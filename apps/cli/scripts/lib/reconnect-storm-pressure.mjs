import assert from "node:assert/strict"

export function assertOnlySlowSubscriptionClosed(relayHealth, clientCount, healthySeen, marker) {
  const healthySubscriptionCount = clientCount - 1
  assert.equal(
    relayHealth.subscription_count,
    healthySubscriptionCount,
    `relay must retain all ${healthySubscriptionCount} healthy subscribers after isolating the slow subscriber`,
  )
  assert.equal(healthySeen.length, healthySubscriptionCount)
  for (const [index, markers] of healthySeen.entries()) {
    assert.ok(markers.has(marker), `healthy subscriber ${index + 1} did not receive the post-close marker`)
  }
}

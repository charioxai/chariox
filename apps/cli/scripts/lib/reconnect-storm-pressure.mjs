import assert from "node:assert/strict"

export function assertOnlySlowSubscriptionClosed(relayHealth, clientCount) {
  const healthySubscriptionCount = clientCount - 1
  assert.equal(
    relayHealth.subscription_count,
    healthySubscriptionCount,
    `relay must retain all ${healthySubscriptionCount} healthy subscribers after isolating the slow subscriber`,
  )
}

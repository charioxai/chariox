import assert from "node:assert/strict"
import { test } from "node:test"
import { replyWith, startFakeKernel } from "./fake-unix-kernel.test-support.js"
import { runNotificationSubscriptionCommand } from "./notification-subscription-command.js"

test("notification subscription verbs use the owner's existing list and pause requests", async () => {
  const subscription = { subscription_id: "sub-1" }
  let paused = false
  const kernel = await startFakeKernel((socket, frame) => {
    const update = frame.request.UpdateNotificationSubscription as { change: { paused: { paused: boolean } } } | undefined
    if (update) paused = update.change.paused.paused
    replyWith(socket, frame, { response: { NotificationSubscriptions: { subscriptions: [{ subscription, paused }], responsibilities: [] } } })
  })
  const original = console.log
  const output: unknown[] = []
  console.log = line => output.push(JSON.parse(line))
  try {
    for (const args of [["list"], ["pause", "sub-1"], ["resume", "sub-1"]]) {
      assert(await runNotificationSubscriptionCommand(["notifications", ...args, "--socket", kernel.path]))
    }
    assert.deepEqual(kernel.frames.map(frame => frame.request), [
      { ListNotificationSubscriptions: null },
      { UpdateNotificationSubscription: { subscription_id: "sub-1", change: { paused: { paused: true } } } },
      { UpdateNotificationSubscription: { subscription_id: "sub-1", change: { paused: { paused: false } } } },
    ])
    assert.deepEqual(output, [
      [{ subscription, paused: false }],
      { subscription_id: "sub-1", paused: true },
      { subscription_id: "sub-1", paused: false },
    ])
  } finally {
    console.log = original
    await kernel.close()
  }
})

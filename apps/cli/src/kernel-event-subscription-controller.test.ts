import assert from "node:assert/strict"
import test from "node:test"

import { createKernelEventSubscriptionController } from "./kernel-event-subscription-controller.js"

test("sync subscribes to waiting-room inventory once while detached", async () => {
  let waitingRoomSubscriptions = 0
  const evaluations: unknown[] = []
  const controller = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => true,
    getAttachment: () => null,
    getSessionId: () => "session-1",
    subscribeToWaitingRoomInventory: async () => {
      waitingRoomSubscriptions += 1
    },
    subscribeToKernelEvents: async () => {
      throw new Error("unexpected session subscription")
    },
    onEvaluate: (state) => {
      evaluations.push(state)
    },
    onWaitingRoomSubscribed: () => {},
    onSessionSubscribed: () => {},
    onWaitingRoomSubscriptionFailed: () => {},
    onSessionSubscriptionFailed: () => {},
  })

  await controller.sync()
  await controller.sync()

  assert.equal(waitingRoomSubscriptions, 1)
  assert.deepEqual(controller.state(), {
    scope: "waiting-room",
    sessionId: null,
    attachmentId: null,
  })
  assert.equal(evaluations.length, 2)
})

test("sync subscribes to session events and skips duplicate bindings", async () => {
  let currentSessionId = "session-1"
  let currentAttachment = { id: "attachment-1" }
  const subscriptions: Array<{ sessionId: string; attachmentId: string }> = []
  const controller = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => true,
    getAttachment: () => currentAttachment,
    getSessionId: () => currentSessionId,
    subscribeToWaitingRoomInventory: async () => {
      throw new Error("unexpected waiting-room subscription")
    },
    subscribeToKernelEvents: async (sessionId, attachmentId) => {
      subscriptions.push({ sessionId, attachmentId })
    },
    onEvaluate: () => {},
    onWaitingRoomSubscribed: () => {},
    onSessionSubscribed: () => {},
    onWaitingRoomSubscriptionFailed: () => {},
    onSessionSubscriptionFailed: () => {},
  })

  await controller.sync()
  await controller.sync()
  currentAttachment = { id: "attachment-2" }
  await controller.sync()

  assert.deepEqual(subscriptions, [
    { sessionId: "session-1", attachmentId: "attachment-1" },
    { sessionId: "session-1", attachmentId: "attachment-2" },
  ])
  assert.deepEqual(controller.state(), {
    scope: "session",
    sessionId: "session-1",
    attachmentId: "attachment-2",
  })
})

test("reset clears the remembered subscription so the same binding resubscribes", async () => {
  let subscriptions = 0
  const controller = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => true,
    getAttachment: () => ({ id: "attachment-1" }),
    getSessionId: () => "session-1",
    subscribeToWaitingRoomInventory: async () => {},
    subscribeToKernelEvents: async () => {
      subscriptions += 1
    },
    onEvaluate: () => {},
    onWaitingRoomSubscribed: () => {},
    onSessionSubscribed: () => {},
    onWaitingRoomSubscriptionFailed: () => {},
    onSessionSubscriptionFailed: () => {},
  })

  await controller.sync()
  controller.reset()
  await controller.sync()

  assert.equal(subscriptions, 2)
})

test("sync reports subscription failures without updating remembered scope", async () => {
  let failure: unknown
  const controller = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => true,
    getAttachment: () => ({ id: "attachment-1" }),
    getSessionId: () => "session-1",
    subscribeToWaitingRoomInventory: async () => {},
    subscribeToKernelEvents: async () => {
      throw new Error("subscribe failed")
    },
    onEvaluate: () => {},
    onWaitingRoomSubscribed: () => {},
    onSessionSubscribed: () => {},
    onWaitingRoomSubscriptionFailed: () => {},
    onSessionSubscriptionFailed: (_sessionId, _attachmentId, error) => {
      failure = error
    },
  })

  await controller.sync()

  assert.match(failure instanceof Error ? failure.message : String(failure), /subscribe failed/)
  assert.deepEqual(controller.state(), {
    scope: null,
    sessionId: null,
    attachmentId: null,
  })
})

test("sync is idle when kernel event streams are unavailable", async () => {
  let evaluated = false
  const controller = createKernelEventSubscriptionController({
    supportsKernelEventStream: () => false,
    getAttachment: () => ({ id: "attachment-1" }),
    getSessionId: () => "session-1",
    subscribeToWaitingRoomInventory: async () => {},
    subscribeToKernelEvents: async () => {},
    onEvaluate: () => {
      evaluated = true
    },
    onWaitingRoomSubscribed: () => {},
    onSessionSubscribed: () => {},
    onWaitingRoomSubscriptionFailed: () => {},
    onSessionSubscriptionFailed: () => {},
  })

  await controller.sync()

  assert.equal(evaluated, false)
  assert.deepEqual(controller.state(), {
    scope: null,
    sessionId: null,
    attachmentId: null,
  })
})

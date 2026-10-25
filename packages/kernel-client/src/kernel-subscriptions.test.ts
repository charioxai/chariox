import assert from "node:assert/strict"
import test from "node:test"

import {
  buildKernelSubscriptionTransportRequest,
  createKernelSessionSubscriptionStart,
  createWaitingRoomInventorySubscriptionStart,
  kernelSubscriptionScopeValue,
  type KernelSubscriptionState,
} from "./kernel-subscriptions.js"

test("kernel session subscription resumes only matching session attachments", () => {
  const previous: KernelSubscriptionState = {
    sessionId: "session-a",
    attachmentId: "attachment-a",
    scope: "session",
    relaySubscriptionId: null,
    relayPublicKey: null,
    relayDecryptEvent: null,
  }

  const resumed = createKernelSessionSubscriptionStart({
    previous,
    lastReceivedEventId: 42,
    sessionId: "session-a",
    attachmentId: "attachment-a",
    relaySubscriptionId: "relay-subscription-a",
  })
  assert.equal(resumed.resumeFromEventId, 42)
  assert.equal(resumed.resetLastReceivedEventId, false)
  assert.equal(resumed.subscription.relaySubscriptionId, "relay-subscription-a")

  const restarted = createKernelSessionSubscriptionStart({
    previous,
    lastReceivedEventId: 42,
    sessionId: "session-b",
    attachmentId: "attachment-a",
    relaySubscriptionId: null,
  })
  assert.equal(restarted.resumeFromEventId, null)
  assert.equal(restarted.resetLastReceivedEventId, true)
})

test("waiting room inventory subscription owns sentinel identity and scope", () => {
  const previous: KernelSubscriptionState = {
    sessionId: "__waiting_room_inventory__",
    attachmentId: "__waiting_room_inventory__",
    scope: "waiting_room_inventory",
    relaySubscriptionId: null,
    relayPublicKey: null,
    relayDecryptEvent: null,
  }

  const start = createWaitingRoomInventorySubscriptionStart({
    previous,
    lastReceivedEventId: 7,
    relaySubscriptionId: "relay-subscription-waiting",
  })

  assert.equal(start.resumeFromEventId, 7)
  assert.equal(start.resetLastReceivedEventId, false)
  assert.equal(start.subscription.sessionId, "__waiting_room_inventory__")
  assert.equal(start.subscription.attachmentId, "__waiting_room_inventory__")
  assert.equal(kernelSubscriptionScopeValue(start.subscription), "waiting_room_inventory")
})

test("kernel subscription transport request preserves scope and resume cursor", () => {
  const request = buildKernelSubscriptionTransportRequest({
    sessionId: "__waiting_room_inventory__",
    attachmentId: "__waiting_room_inventory__",
    scope: "waiting_room_inventory",
    relaySubscriptionId: null,
    relayPublicKey: null,
    relayDecryptEvent: null,
  }, 9)

  assert.deepEqual(request, {
    __kernel_transport: {
      type: "subscribe",
      session_id: "__waiting_room_inventory__",
      attachment_id: "__waiting_room_inventory__",
      subscription_scope: "waiting_room_inventory",
      resume_from_event_id: 9,
    },
  })
})

test("a subscription that asks for a heartbeat interval sends it on every subscribe", () => {
  const session = createKernelSessionSubscriptionStart({
    previous: null,
    lastReceivedEventId: null,
    sessionId: "session-a",
    attachmentId: "attachment-a",
    relaySubscriptionId: null,
    heartbeatIntervalMs: 1000,
  })
  const control = buildKernelSubscriptionTransportRequest(session.subscription, null).__kernel_transport
  assert.equal("heartbeat_interval_ms" in control ? control.heartbeat_interval_ms : undefined, 1000)
  const waitingRoom = createWaitingRoomInventorySubscriptionStart({
    previous: null,
    lastReceivedEventId: null,
    relaySubscriptionId: null,
    heartbeatIntervalMs: 1000,
  })
  assert.equal(waitingRoom.subscription.heartbeatIntervalMs, 1000)
  const quiet = createWaitingRoomInventorySubscriptionStart({
    previous: null,
    lastReceivedEventId: null,
    relaySubscriptionId: null,
  })
  assert.equal("heartbeat_interval_ms" in buildKernelSubscriptionTransportRequest(quiet.subscription, null).__kernel_transport, false)
})

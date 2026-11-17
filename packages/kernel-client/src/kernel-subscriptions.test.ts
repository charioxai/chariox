import assert from "node:assert/strict"
import test from "node:test"

import {
  buildKernelSubscriptionTransportRequest,
  createKernelSessionSubscriptionStart,
  createRoomListSubscriptionStart,
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

test("Room list inventory subscription owns sentinel identity and scope", () => {
  const previous: KernelSubscriptionState = {
    sessionId: "__room_list__",
    attachmentId: "__room_list__",
    scope: "room_list",
    relaySubscriptionId: null,
    relayPublicKey: null,
    relayDecryptEvent: null,
  }

  const start = createRoomListSubscriptionStart({
    previous,
    lastReceivedEventId: 7,
    relaySubscriptionId: "relay-subscription-waiting",
  })

  assert.equal(start.resumeFromEventId, 7)
  assert.equal(start.resetLastReceivedEventId, false)
  assert.equal(start.subscription.sessionId, "__room_list__")
  assert.equal(start.subscription.attachmentId, "__room_list__")
  assert.equal(kernelSubscriptionScopeValue(start.subscription), "room_list")
})

test("kernel subscription transport request preserves scope and resume cursor", () => {
  const request = buildKernelSubscriptionTransportRequest({
    sessionId: "__room_list__",
    attachmentId: "__room_list__",
    scope: "room_list",
    relaySubscriptionId: null,
    relayPublicKey: null,
    relayDecryptEvent: null,
  }, 9)

  assert.deepEqual(request, {
    __kernel_transport: {
      type: "subscribe",
      session_id: "__room_list__",
      attachment_id: "__room_list__",
      subscription_scope: "room_list",
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
  const roomList = createRoomListSubscriptionStart({
    previous: null,
    lastReceivedEventId: null,
    relaySubscriptionId: null,
    heartbeatIntervalMs: 1000,
  })
  assert.equal(roomList.subscription.heartbeatIntervalMs, 1000)
  const quiet = createRoomListSubscriptionStart({
    previous: null,
    lastReceivedEventId: null,
    relaySubscriptionId: null,
  })
  assert.equal("heartbeat_interval_ms" in buildKernelSubscriptionTransportRequest(quiet.subscription, null).__kernel_transport, false)
})

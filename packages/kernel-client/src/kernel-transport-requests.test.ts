import assert from "node:assert/strict"
import test from "node:test"

import { extractTransportRequest, normalizeWebSocketRequest } from "./kernel-transport-requests.js"

test("normalizeWebSocketRequest wraps regular daemon requests", () => {
  const request = { ListSessions: null }

  assert.deepEqual(normalizeWebSocketRequest("request-1", request), {
    type: "request",
    request_id: "request-1",
    command_id: "request-1",
    request,
  })
})

test("normalizeWebSocketRequest projects kernel subscribe controls", () => {
  assert.deepEqual(normalizeWebSocketRequest("request-2", {
    __kernel_transport: {
      type: "subscribe",
      session_id: "session-1",
      attachment_id: "attachment-1",
      subscription_scope: "waiting_room_inventory",
      resume_from_event_id: 42,
    },
  }), {
    type: "subscribe",
    request_id: "request-2",
    session_id: "session-1",
    attachment_id: "attachment-1",
    subscription_scope: "waiting_room_inventory",
    resume_from_event_id: 42,
  })
})

test("normalizeWebSocketRequest defaults optional subscribe controls to null", () => {
  assert.deepEqual(normalizeWebSocketRequest("request-3", {
    __kernel_transport: {
      type: "subscribe",
      session_id: "session-1",
      attachment_id: "attachment-1",
    },
  }), {
    type: "subscribe",
    request_id: "request-3",
    session_id: "session-1",
    attachment_id: "attachment-1",
    subscription_scope: null,
    resume_from_event_id: null,
  })
})

test("extractTransportRequest rejects malformed transport controls", () => {
  assert.equal(extractTransportRequest(null), null)
  assert.equal(extractTransportRequest({ __kernel_transport: { type: "subscribe" } }), null)
  assert.deepEqual(extractTransportRequest({ __kernel_transport: { type: "unsubscribe" } }), {
    type: "unsubscribe",
  })
})

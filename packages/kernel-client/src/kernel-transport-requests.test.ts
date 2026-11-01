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

test("normalizeWebSocketRequest carries a target kernel and caller command id", () => {
  assert.deepEqual(normalizeWebSocketRequest("request-3", { ListSessions: null }, { kernel: "b3", commandId: "cmd-1" }), {
    type: "request",
    request_id: "request-3",
    command_id: "cmd-1",
    request: { ListSessions: null },
    kernel: "b3",
  })
  assert.deepEqual(normalizeWebSocketRequest("request-4", { ListSessions: null }, { kernel: "b3" }), {
    type: "request",
    request_id: "request-4",
    command_id: "request-4",
    request: { ListSessions: null },
    kernel: "b3",
  })
  assert.equal("kernel" in normalizeWebSocketRequest("request-5", { ListSessions: null }, { commandId: "cmd-2" }), false)
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

test("normalizeWebSocketRequest keeps a requested heartbeat interval on subscribe controls", () => {
  const control = { type: "subscribe", session_id: "session-1", attachment_id: "attachment-1" }
  assert.equal(
    "heartbeat_interval_ms" in normalizeWebSocketRequest("request-4", { __kernel_transport: control }),
    false,
  )
  assert.deepEqual(
    normalizeWebSocketRequest("request-5", { __kernel_transport: { ...control, heartbeat_interval_ms: 1000 } }),
    {
      type: "subscribe",
      request_id: "request-5",
      session_id: "session-1",
      attachment_id: "attachment-1",
      subscription_scope: null,
      resume_from_event_id: null,
      heartbeat_interval_ms: 1000,
    },
  )
})

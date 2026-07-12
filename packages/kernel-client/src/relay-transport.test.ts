import assert from "node:assert/strict"
import test from "node:test"

import {
  createRelayKeypair,
  decryptRelayPayload,
  relayPublicKeyFromPrivateKey,
} from "./relay-crypto.js"
import {
  buildRelaySubscribeFrame,
  buildRelayUnsubscribeFrame,
  normalizeRelayRequest,
  requireRelayTarget,
} from "./relay-transport.js"

test("relay requests preserve the request id as the kernel command id", () => {
  const daemon = createRelayKeypair()
  const request = {
    PumpTerminalOutput: {
      session_id: "session-1",
      attachment_id: "attachment-1",
    },
  }
  const normalized = normalizeRelayRequest(
    "request-1",
    request,
    { daemon_id: "daemon-1", daemon_alias: null },
    daemon.publicKeyBase64,
  )

  assert.deepEqual(
    JSON.parse(decryptRelayPayload(daemon.privateKey, normalized.frame.encrypted_request)),
    {
      command_id: "request-1",
      request,
    },
  )
})

test("buildRelaySubscribeFrame projects scoped relay subscriptions", () => {
  const frame = buildRelaySubscribeFrame({
    requestId: "request-1",
    subscriptionId: "subscription-1",
    target: { daemon_id: "daemon-1", daemon_alias: null },
    sessionId: "session-1",
    attachmentId: "attachment-1",
    clientPublicKey: "client-public-key",
    resumeFromEventId: 42,
    subscriptionScope: "waiting_room_inventory",
  })

  assert.deepEqual(frame, {
    kind: "client_subscribe",
    request_id: "request-1",
    subscription_id: "subscription-1",
    target: { daemon_id: "daemon-1", daemon_alias: null },
    session_id: "session-1",
    attachment_id: "attachment-1",
    client_public_key: "client-public-key",
    resume_from_event_id: 42,
    subscription_scope: "waiting_room_inventory",
  })
})

test("buildRelaySubscribeFrame omits absent subscription scope", () => {
  const frame = buildRelaySubscribeFrame({
    requestId: "request-1",
    subscriptionId: "subscription-1",
    target: { daemon_id: null, daemon_alias: "machine" },
    sessionId: "session-1",
    attachmentId: "attachment-1",
    clientPublicKey: "client-public-key",
    resumeFromEventId: null,
  })

  assert.equal(frame.subscription_scope, undefined)
})

test("buildRelayUnsubscribeFrame derives the client public key", () => {
  const keypair = createRelayKeypair()

  assert.deepEqual(buildRelayUnsubscribeFrame("request-1", "subscription-1", keypair.privateKey), {
    kind: "client_unsubscribe",
    request_id: "request-1",
    subscription_id: "subscription-1",
    client_public_key: relayPublicKeyFromPrivateKey(keypair.privateKey),
  })
})

test("relay subscription frames require a target", () => {
  assert.throws(() => requireRelayTarget(null), /relay target daemon id or alias is required/)
  assert.throws(() => buildRelaySubscribeFrame({
    requestId: "request-1",
    subscriptionId: "subscription-1",
    target: {},
    sessionId: "session-1",
    attachmentId: "attachment-1",
    clientPublicKey: "client-public-key",
    resumeFromEventId: null,
  }), /relay target daemon id or alias is required/)
})

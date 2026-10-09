import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { respondToInteractionRequest } from "./ipc-terminal-runtime-requests.js"

test("protocol 392: a critical approval carries the passkey and an optional remember window", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 481)
  assert.deepEqual(respondToInteractionRequest("s", "i", "deny"), {
    RespondToInteraction: { session_id: "s", interaction_id: "i", choice_id: "deny", custom_reply: null },
  })
  assert.deepEqual(respondToInteractionRequest("s", "i", "approve", null, { passkey: "pk", rememberMinutes: 15 }), {
    RespondToInteraction: {
      session_id: "s", interaction_id: "i", choice_id: "approve", custom_reply: null,
      passkey: "pk", passkey_remember_minutes: 15,
    },
  })
  assert.equal("passkey_remember_minutes" in respondToInteractionRequest("s", "i", "approve", null, { passkey: "pk" }).RespondToInteraction, false)
})

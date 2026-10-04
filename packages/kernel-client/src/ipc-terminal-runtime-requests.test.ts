import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { activePromptSteeringMinimumProtocolVersion, respondToInteractionRequest, steerActivePromptRequest } from "./ipc-terminal-runtime-requests.js"

test("MP-08 MP-10 protocol 422 atomically admits steering against a home prompt", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 422)
  assert.equal(activePromptSteeringMinimumProtocolVersion, 422)
  assert.deepEqual(steerActivePromptRequest("s", "a", "agent", "home-prompt", "new direction", []), {
    SteerActivePrompt: { session_id: "s", attachment_id: "a", target_agent_id: "agent",
      expected_active_prompt_id: "home-prompt", prompt: "new direction", attachments: [] },
  })
})

test("protocol 392: a critical approval carries the passkey and an optional remember window", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 422)
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

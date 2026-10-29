import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { activePromptSteeringMinimumProtocolVersion, respondToInteractionRequest, steerActivePromptRequest } from "./ipc-terminal-runtime-requests.js"

test("MP-08 MP-10 protocol 491 atomically admits steering against a home prompt", () => {
  assert.equal(activePromptSteeringMinimumProtocolVersion, 491)
  assert.deepEqual(steerActivePromptRequest("s", "a", "agent", "home-prompt", "new direction", []), {
    SteerActivePrompt: { session_id: "s", attachment_id: "a", target_agent_id: "agent",
      expected_active_prompt_id: "home-prompt", prompt: "new direction", attachments: [] },
  })
})

test("protocol 392: a critical approval carries the passkey and an optional remember window", () => {

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

// MP-08 / MP-10 / MP-11: preserve legacy cancellation, pin the new optional field.
test("MP-08 MP-10 exact cancellation forwards the home prompt with protocol499", async () => {
  const r = await import("./ipc-terminal-runtime-requests.js")
  assert.equal(r.exactPromptCancellationMinimumProtocolVersion, 491)
  assert.equal(r.cancelActivePromptRequest("s", "a", "agent", "").CancelActivePrompt.expected_prompt_id, "")
  assert.deepEqual(r.cancelActivePromptRequest("s", "a", "agent", "prompt"), {
    CancelActivePrompt: { session_id: "s", attachment_id: "a", target_agent_id: "agent", expected_prompt_id: "prompt" },
  })
  assert.deepEqual(r.cancelActivePromptRequest("s", "a", "agent"), {
    CancelActivePrompt: { session_id: "s", attachment_id: "a", target_agent_id: "agent" },
  })
})

test("MP-08/MP-10/MP-11: HTTP trigger client contracts preserve payload and history cursor", async () => {
  const r = await import("./ipc-terminal-runtime-requests.js")
  assert.deepEqual(r.createAgentHttpTriggerRequest({ sessionId: "s", agentId: "a", busyPolicy: "steer", prompt: " Input " }), {
    CreateAgentTrigger: { session_id: "s", agent_id: "a", source: { kind: "http" }, busy_policy: "steer", prompt: "Input" },
  })
  const payload = { prompt: "λ </script>", query: { repeated: ["one", "two"] } }
  assert.deepEqual(r.invokeAgentTriggerRequest("s", "t", payload), { InvokeAgentTrigger: { session_id: "s", trigger_id: "t", payload } })
  assert.deepEqual(r.readAgentTriggerTranscriptRequest("s", "t", { before_sequence: 100 }), {
    ReadAgentTriggerTranscript: { session_id: "s", trigger_id: "t", cursor: { before_sequence: 100 } },
  })
})

test("protocol 901 resends a trigger firing the provider did not confirm", async () => {
  const r = await import("./ipc-terminal-runtime-requests.js")
  assert.equal(r.triggerDeliveryMinimumProtocolVersion, 901)
  assert.deepEqual(r.resendTriggerDeliveryRequest("s", "prompt-7"), {
    ResendTriggerDelivery: { session_id: "s", prompt_id: "prompt-7" },
  })
})

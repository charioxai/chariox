import assert from "node:assert/strict"
import test from "node:test"

import { providerTranscriptRoleForKind } from "./transcript-kind-role.js"

test("provider transcript role mapping is shared by live and history projection", () => {
  assert.equal(providerTranscriptRoleForKind("provider_output"), "assistant")
  assert.equal(providerTranscriptRoleForKind("provider_reasoning"), "reasoning")
  assert.equal(providerTranscriptRoleForKind("provider_tool"), "tool")
  assert.equal(providerTranscriptRoleForKind("provider_error"), "error")
  assert.equal(providerTranscriptRoleForKind("provider_status"), "status")
  assert.equal(providerTranscriptRoleForKind("prompt_echo"), null)
  assert.equal(providerTranscriptRoleForKind("user_prompt"), null)
  assert.equal(providerTranscriptRoleForKind("notice"), null)
})

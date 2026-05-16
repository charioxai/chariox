import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeInteraction } from "./cli-types.js"
import {
  appendInteractionCustomReply,
  interactionCustomChoiceIndex,
  nextInteractionChoiceIndex,
  resolveInteractionChoiceSubmission,
  shouldEditCustomInteractionOnEnter,
} from "./interaction-choice-state.js"

test("resolveInteractionChoiceSubmission submits regular choices", () => {
  assert.deepEqual(resolveInteractionChoiceSubmission({
    interaction: interaction(),
    selectedIndex: 1,
    customReply: "",
  }), {
    action: "submit",
    selectedIndex: 1,
    choiceId: "deny",
    customReply: null,
  })
})

test("resolveInteractionChoiceSubmission validates custom replies", () => {
  const source = interaction({
    custom_choice: {
      id: "custom",
      label: "Custom",
      min_length: 3,
      max_length: 10,
    },
  })

  assert.deepEqual(resolveInteractionChoiceSubmission({
    interaction: source,
    requestedIndex: 99,
    customReply: "ok",
  }), {
    action: "edit_custom",
  })
  assert.deepEqual(resolveInteractionChoiceSubmission({
    interaction: source,
    requestedIndex: 99,
    customReply: "ship",
  }), {
    action: "submit",
    selectedIndex: 2,
    choiceId: "custom",
    customReply: "ship",
  })
})

test("resolveInteractionChoiceSubmission reports unavailable empty interactions", () => {
  assert.deepEqual(resolveInteractionChoiceSubmission({
    interaction: interaction({ choices: [] }),
    selectedIndex: 0,
    customReply: "",
  }), {
    action: "unavailable",
  })
})

test("interaction choice helpers handle cycling and custom indexes", () => {
  const source = interaction({
    custom_choice: {
      id: "custom",
      label: "Custom",
    },
  })
  assert.equal(interactionCustomChoiceIndex(source), 2)
  assert.equal(nextInteractionChoiceIndex({ interaction: source, currentIndex: 2, delta: 1 }), 0)
  assert.equal(nextInteractionChoiceIndex({ interaction: source, currentIndex: 0, delta: -1 }), 2)
  assert.equal(nextInteractionChoiceIndex({
    interaction: interaction({ choices: [] }),
    currentIndex: 0,
    delta: 1,
  }), null)
})

test("custom reply helpers respect editing thresholds", () => {
  const source = interaction({
    custom_choice: {
      id: "custom",
      label: "Custom",
    },
  })
  assert.equal(appendInteractionCustomReply({ current: "ab", input: "c", maxLength: 3 }), "abc")
  assert.equal(appendInteractionCustomReply({ current: "abc", input: "d", maxLength: 3 }), "abc")
  assert.equal(shouldEditCustomInteractionOnEnter({
    interaction: source,
    selectedIndex: 2,
    customReply: "",
  }), true)
  assert.equal(shouldEditCustomInteractionOnEnter({
    interaction: source,
    selectedIndex: 2,
    customReply: "custom",
  }), false)
})

function interaction(overrides: Partial<RuntimeInteraction> = {}): RuntimeInteraction {
  return {
    id: "interaction-1",
    agent_id: "agent-1",
    kind: "choice",
    level: "info",
    title: null,
    message: "Choose",
    choices: [
      { id: "allow", label: "Allow", reply: "yes" },
      { id: "deny", label: "Deny", reply: "no" },
    ],
    custom_choice: null,
    timeout_sec: null,
    default_on_timeout: null,
    requested_at_ms: 1,
    ...overrides,
  }
}

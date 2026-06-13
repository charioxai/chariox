import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeInteraction } from "./cli-types.js"
import {
  appendInteractionCustomReply,
  deleteInteractionCustomReply,
  interactionCustomChoiceIndex,
  nextInteractionChoiceIndex,
  resolveInteractionChoiceKeyAction,
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

test("resolveInteractionChoiceSubmission can attach custom replies to fixed choices", () => {
  assert.deepEqual(resolveInteractionChoiceSubmission({
    interaction: interaction({
      custom_choice: {
        id: "passphrase",
        label: "Passphrase",
        input_kind: "secret",
      },
    }),
    selectedIndex: 0,
    customReply: "vault-passphrase",
  }), {
    action: "submit",
    selectedIndex: 0,
    choiceId: "allow",
    customReply: "vault-passphrase",
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
  assert.equal(deleteInteractionCustomReply("abc"), "ab")
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

test("resolveInteractionChoiceKeyAction handles custom reply editing keys", () => {
  const source = interaction({
    custom_choice: {
      id: "custom",
      label: "Custom",
    },
  })

  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "a" },
    selectedIndex: 2,
    customEditing: true,
    customReply: "",
  }), {
    action: "append_custom_reply",
    input: "a",
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "backspace" },
    selectedIndex: 2,
    customEditing: true,
    customReply: "abc",
  }), {
    action: "delete_custom_reply",
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "escape" },
    selectedIndex: 2,
    customEditing: true,
    customReply: "abc",
  }), {
    action: "cancel_custom_edit",
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "tab" },
    selectedIndex: 2,
    customEditing: true,
    customReply: "abc",
  }), {
    action: "handled",
    consumeEvent: false,
  })
})

test("resolveInteractionChoiceKeyAction handles navigation and submission keys", () => {
  const source = interaction({
    custom_choice: {
      id: "custom",
      label: "Custom",
    },
  })

  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "left" },
    selectedIndex: 0,
    customEditing: false,
    customReply: "",
  }), {
    action: "cycle",
    delta: -1,
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "3" },
    selectedIndex: 0,
    customEditing: false,
    customReply: "",
  }), {
    action: "begin_custom_edit",
    selectedIndex: 2,
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "1" },
    selectedIndex: 0,
    customEditing: false,
    customReply: "",
  }), {
    action: "submit",
    choiceIndex: 0,
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "enter" },
    selectedIndex: 2,
    customEditing: false,
    customReply: "",
  }), {
    action: "begin_custom_edit",
    selectedIndex: 2,
    consumeEvent: true,
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: source,
    event: { name: "enter" },
    selectedIndex: 2,
    customEditing: false,
    customReply: "custom",
  }), {
    action: "submit",
    consumeEvent: true,
  })
})

test("resolveInteractionChoiceKeyAction ignores releases and unrelated keys", () => {
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: interaction(),
    event: { name: "enter", eventType: "release" },
    selectedIndex: 0,
    customEditing: false,
    customReply: "",
  }), {
    action: "ignore",
  })
  assert.deepEqual(resolveInteractionChoiceKeyAction({
    interaction: interaction(),
    event: { name: "x" },
    selectedIndex: 0,
    customEditing: false,
    customReply: "",
  }), {
    action: "ignore",
  })
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

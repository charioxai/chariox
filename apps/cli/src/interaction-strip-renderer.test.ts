import assert from "node:assert/strict"
import test from "node:test"

import { renderInteractionCustomChoiceValue } from "./interaction-custom-choice-render.js"

test("interaction strip masks secret custom choice values", () => {
  assert.equal(
    renderInteractionCustomChoiceValue("super-secret-password", "Password", "secret"),
    "*********************",
  )
  assert.equal(
    renderInteractionCustomChoiceValue("visible answer", "Answer", "text"),
    "visible answer",
  )
  assert.equal(
    renderInteractionCustomChoiceValue("", "Password", "secret"),
    "<Password>",
  )
})

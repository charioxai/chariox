import assert from "node:assert/strict"
import test from "node:test"

import {
  EXTERNAL_PROMPT_ORIGIN,
  normalizePromptOrigin,
  promptOriginIsExternal,
} from "./prompt-origin.js"

test("prompt origin helpers normalize serialized kernel origin values", () => {
  assert.equal(normalizePromptOrigin(" External "), EXTERNAL_PROMPT_ORIGIN)
  assert.equal(normalizePromptOrigin(""), null)
  assert.equal(normalizePromptOrigin("   "), null)
  assert.equal(promptOriginIsExternal(" External "), true)
  assert.equal(promptOriginIsExternal("arroba"), false)
  assert.equal(promptOriginIsExternal(null), false)
})

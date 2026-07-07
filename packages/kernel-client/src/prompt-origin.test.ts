import assert from "node:assert/strict"
import test from "node:test"

import {
  ARROBA_PROMPT_ORIGIN,
  EXTERNAL_PROMPT_ORIGIN,
  normalizePromptOrigin,
  promptOriginFromRecord,
  promptOriginIsExternal,
} from "./prompt-origin.js"

test("prompt origin helpers normalize serialized kernel origin values", () => {
  assert.equal(normalizePromptOrigin(" External "), EXTERNAL_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord({ prompt_origin: " External " }), EXTERNAL_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord({ prompt_origin: "external" }), EXTERNAL_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord({ prompt_origin: "arroba" }), ARROBA_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord({}, ARROBA_PROMPT_ORIGIN), ARROBA_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord({ prompt_origin: " arroba " }), ARROBA_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord({ prompt_origin: "   " }, ARROBA_PROMPT_ORIGIN), ARROBA_PROMPT_ORIGIN)
  assert.equal(promptOriginFromRecord(null), null)
  assert.equal(normalizePromptOrigin(""), null)
  assert.equal(normalizePromptOrigin("   "), null)
  assert.equal(promptOriginIsExternal(" External "), true)
  assert.equal(promptOriginIsExternal("arroba"), false)
  assert.equal(promptOriginIsExternal(null), false)
})

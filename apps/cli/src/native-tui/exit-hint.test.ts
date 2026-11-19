import assert from "node:assert/strict"
import test from "node:test"

import { formatNativeReopenHint } from "./exit-hint.js"
import { NATIVE_PROMPT_POLICY } from "./runtime-banner.js"

test("the exit hint is the Chariox command, never the provider's resume command", () => {
  for (const provider of ["claude", "codex", "opencode"] as const) {
    const hint = formatNativeReopenHint(provider, "18e925926ce97bc2")
    assert.match(hint, new RegExp(`chariox ${provider} 18e925926ce97bc2`))
    assert.doesNotMatch(hint, /--resume|resume|-s /)
  }
})

test("the banner tells each provider's prompt path as it is", () => {
  assert.match(NATIVE_PROMPT_POLICY.observed, /straight to the provider/)
  assert.match(NATIVE_PROMPT_POLICY.throughKernel, /through the Chariox kernel/)
  assert.notEqual(NATIVE_PROMPT_POLICY.observed, NATIVE_PROMPT_POLICY.throughKernel)
})

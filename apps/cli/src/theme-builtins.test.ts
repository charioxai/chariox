import assert from "node:assert/strict"
import test from "node:test"

import { BUILTIN_THEME_DEFINITIONS } from "./theme-builtins.js"
import { THEME_NAMES, THEME_TOKEN_KEYS } from "./theme-contracts.js"

test("built-in theme catalog follows the advertised theme order", () => {
  assert.deepEqual(
    BUILTIN_THEME_DEFINITIONS.map((theme) => theme.id),
    [...THEME_NAMES],
  )
})

test("built-in theme catalog provides all theme tokens", () => {
  for (const theme of BUILTIN_THEME_DEFINITIONS) {
    assert.equal(Object.keys(theme.colors).length, THEME_TOKEN_KEYS.length)
    for (const key of THEME_TOKEN_KEYS) {
      assert.match(theme.colors[key], /^#[0-9a-f]{6}$/)
    }
  }
})

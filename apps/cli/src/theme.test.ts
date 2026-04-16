import assert from "node:assert/strict"
import test from "node:test"

import {
  normalizeThemeName,
  themeLabel,
  themeOptions,
} from "./theme-registry.js"

test("theme registry normalizes and labels waiting room options", () => {
  assert.equal(normalizeThemeName("sober"), "sober")
  assert.equal(normalizeThemeName("missing"), "opencode")
  assert.equal(themeLabel("tokyonight"), "Tokyonight")
  assert.equal(themeOptions().some((option) => option.id === "sober"), true)
})

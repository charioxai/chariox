import assert from "node:assert/strict"
import test from "node:test"

import {
  ansiToHex,
  hex,
  isDarkHex,
  mixHex,
  optionalHex,
} from "./theme-color-utils.js"

test("theme color utilities normalize and validate hex colors", () => {
  assert.equal(optionalHex(" #AABBCC "), "#aabbcc")
  assert.equal(optionalHex("red"), null)
  assert.equal(hex("#123456", "primary"), "#123456")
  assert.throws(() => hex("123456", "primary"), /primary must be a #rrggbb color/)
})

test("theme color utilities mix colors and classify dark colors", () => {
  assert.equal(mixHex("#000000", "#ffffff", 0.5), "#808080")
  assert.equal(isDarkHex("#000000"), true)
  assert.equal(isDarkHex("#ffffff"), false)
})

test("theme color utilities convert ANSI colors", () => {
  assert.equal(ansiToHex(9), "#ff0000")
  assert.equal(ansiToHex(16), "#000000")
  assert.equal(ansiToHex(231), "#ffffff")
  assert.equal(ansiToHex(232), "#080808")
  assert.equal(ansiToHex(999), "#000000")
})

import assert from "node:assert/strict"
import test from "node:test"

import { arrobaArtFrame } from "./waiting-room-art.js"

test("arrobaArtFrame resolves to the clean logo after the intro completes", () => {
  const first = arrobaArtFrame(0)
  const last = arrobaArtFrame(12)
  assert.notEqual(first, last)
  assert.equal(last.includes("____"), true)
})

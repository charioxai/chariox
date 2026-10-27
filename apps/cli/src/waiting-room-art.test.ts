import assert from "node:assert/strict"
import test from "node:test"

import { charioxArtFrame } from "./waiting-room-art.js"

test("charioxArtFrame resolves to the clean logo after the intro completes", () => {
  const first = charioxArtFrame(0)
  const last = charioxArtFrame(12)
  assert.notEqual(first, last)
  assert.equal(last.includes("____"), true)
})

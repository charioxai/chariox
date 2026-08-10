import assert from "node:assert/strict"
import test from "node:test"

import {
  canonicalUtcInstant,
  parseAbsoluteInstantMs,
  parseAbsoluteInstantMsOrNull,
} from "./time.js"

test("absolute instants accept UTC and numeric offsets", () => {
  assert.equal(parseAbsoluteInstantMs("2026-01-15T12:00:00.000Z"), 1_768_478_400_000)
  assert.equal(parseAbsoluteInstantMs("2026-01-15T14:00:00+02:00"), 1_768_478_400_000)
  assert.equal(canonicalUtcInstant("2026-01-15T07:00:00-05:00"), "2026-01-15T12:00:00.000Z")
})

test("absolute instants reject timezone-free and invalid values", () => {
  assert.throws(() => parseAbsoluteInstantMs("2026-01-15T12:00:00"), /explicit timezone/)
  assert.throws(() => parseAbsoluteInstantMs("2026-01-15"), /explicit timezone/)
  assert.throws(() => parseAbsoluteInstantMs("not-a-date"), /explicit timezone/)
  assert.equal(parseAbsoluteInstantMsOrNull("2026-01-15T12:00:00"), null)
})

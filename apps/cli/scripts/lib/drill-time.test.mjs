import assert from "node:assert/strict"
import test from "node:test"

import {
  parseDrillIsoTimestamp,
  validateDrillTimestampOrder,
} from "./drill-time.mjs"

test("parses strict drill ISO timestamps", () => {
  assert.equal(
    parseDrillIsoTimestamp("2026-06-14T00:00:00.000Z", "field"),
    Date.parse("2026-06-14T00:00:00.000Z"),
  )
  assert.throws(() => parseDrillIsoTimestamp("", "field"), /missing timestamp/)
  assert.throws(() => parseDrillIsoTimestamp("2026-06-14", "field"), /ISO timestamp/)
  assert.throws(() => parseDrillIsoTimestamp("not-a-date", "field"), /ISO timestamp/)
})

test("validates drill timestamp ordering", () => {
  validateDrillTimestampOrder({
    startedAt: "2026-06-14T00:00:00.000Z",
    completedAt: "2026-06-14T00:00:01.000Z",
  }, "report")

  assert.throws(() => validateDrillTimestampOrder({
    startedAt: "2026-06-14T00:00:01.000Z",
    completedAt: "2026-06-14T00:00:00.000Z",
  }, "report"), /completedAt must not be before startedAt/)
})

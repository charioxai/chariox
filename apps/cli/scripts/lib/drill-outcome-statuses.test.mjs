import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_OUTCOME_STATUSES,
  isKnownDrillOutcomeStatus,
  validateDrillOutcomeStatus,
} from "./drill-outcome-statuses.mjs"

test("validates drill outcome status metadata", () => {
  assert.deepEqual(DRILL_OUTCOME_STATUSES, ["passed", "failed", "skipped", "dry-run"])
  assert.equal(isKnownDrillOutcomeStatus("passed"), true)
  assert.equal(isKnownDrillOutcomeStatus("pending"), false)
  assert.doesNotThrow(() => validateDrillOutcomeStatus("dry-run", "scenario.status"))
  assert.throws(
    () => validateDrillOutcomeStatus("pending", "scenario.status"),
    /scenario\.status has invalid status "pending"/,
  )
  assert.throws(
    () => validateDrillOutcomeStatus("pending", "scenario.status", {
      message: () => "scenario.status is invalid",
    }),
    /scenario\.status is invalid/,
  )
})

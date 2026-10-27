import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_CHECK_STATUSES,
  DRILL_VALIDATION_RESULT_STATUSES,
  isKnownDrillValidationCheckStatus,
  isKnownDrillValidationResultStatus,
  validateDrillValidationCheckStatus,
  validateDrillValidationResultStatus,
} from "./drill-validation-statuses.mjs"

test("validates validation result and check status metadata", () => {
  assert.deepEqual(DRILL_VALIDATION_RESULT_STATUSES, ["passed", "failed"])
  assert.deepEqual(DRILL_VALIDATION_CHECK_STATUSES, ["passed", "failed", "skipped"])
  assert.equal(isKnownDrillValidationResultStatus("passed"), true)
  assert.equal(isKnownDrillValidationResultStatus("skipped"), false)
  assert.equal(isKnownDrillValidationCheckStatus("skipped"), true)
  assert.equal(isKnownDrillValidationCheckStatus("dry-run"), false)
  assert.doesNotThrow(() => validateDrillValidationResultStatus("failed", "report"))
  assert.doesNotThrow(() => validateDrillValidationCheckStatus("skipped", "check"))
  assert.throws(
    () => validateDrillValidationResultStatus("skipped", "report"),
    /report has invalid status "skipped"/,
  )
  assert.throws(
    () => validateDrillValidationCheckStatus("dry-run", "check"),
    /check has invalid status "dry-run"/,
  )
  assert.throws(
    () => validateDrillValidationResultStatus("skipped", "run", {
      message: () => "drill artifact run has invalid status",
    }),
    /drill artifact run has invalid status/,
  )
})

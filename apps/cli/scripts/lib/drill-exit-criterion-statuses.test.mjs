import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_EXIT_CRITERION_STATUSES,
  isKnownDrillExitCriterionStatus,
  validateDrillExitCriterionStatus,
} from "./drill-exit-criterion-statuses.mjs"

test("validates exit criterion status metadata", () => {
  assert.deepEqual(DRILL_EXIT_CRITERION_STATUSES, ["satisfied", "failed", "skipped", "dry-run"])
  assert.equal(isKnownDrillExitCriterionStatus("satisfied"), true)
  assert.equal(isKnownDrillExitCriterionStatus("satisifed"), false)
  assert.doesNotThrow(() => validateDrillExitCriterionStatus("dry-run", "field[0]"))
  assert.throws(
    () => validateDrillExitCriterionStatus("satisifed", "field[0]"),
    /field\[0\] has unknown exit criterion status "satisifed"/,
  )
  assert.throws(
    () => validateDrillExitCriterionStatus("satisifed", "requiredArtifactExitCriterionStatuses", {
      message: (status) => `unknown required artifact exit criterion status: ${status}`,
    }),
    /unknown required artifact exit criterion status: satisifed/,
  )
})

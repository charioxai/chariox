import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_MATRIX_REPORT_STATUSES,
  DRILL_MATRIX_SCENARIO_STATUSES,
  isKnownDrillMatrixReportStatus,
  isKnownDrillMatrixScenarioStatus,
  validateDrillMatrixReportStatus,
  validateDrillMatrixScenarioStatus,
} from "./drill-matrix-statuses.mjs"

test("validates matrix report and scenario status metadata", () => {
  assert.deepEqual(DRILL_MATRIX_REPORT_STATUSES, ["passed", "failed", "dry-run"])
  assert.deepEqual(DRILL_MATRIX_SCENARIO_STATUSES, ["passed", "failed", "skipped", "dry-run"])
  assert.equal(isKnownDrillMatrixReportStatus("passed"), true)
  assert.equal(isKnownDrillMatrixReportStatus("skipped"), false)
  assert.equal(isKnownDrillMatrixScenarioStatus("skipped"), true)
  assert.equal(isKnownDrillMatrixScenarioStatus("pending"), false)
  assert.doesNotThrow(() => validateDrillMatrixReportStatus("dry-run", "report"))
  assert.doesNotThrow(() => validateDrillMatrixScenarioStatus("skipped", "scenario"))
  assert.throws(
    () => validateDrillMatrixReportStatus("skipped", "report"),
    /report has invalid status "skipped"/,
  )
  assert.throws(
    () => validateDrillMatrixScenarioStatus("pending", "scenario"),
    /scenario has invalid status "pending"/,
  )
  assert.throws(
    () => validateDrillMatrixScenarioStatus("pending", "scenario", {
      message: (status) => `scenario rejected matrix status: ${status}`,
    }),
    /scenario rejected matrix status: pending/,
  )
})
